//! The hand-written e2e fixture.
//!
//! Creates a full account hierarchy, account-anchored budgets with initial
//! revisions, and 279 transactions covering 6 historical months plus the
//! current month — including a 150-transaction `Assets:Archive` account used
//! by the register lazy-loading E2E spec.

use bc_core::AccountService;
use bc_core::BudgetService;
use bc_core::TagService;
use bc_core::TransactionService;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::Amount;
use bc_models::BudgetIntent;
use bc_models::BudgetRevision;
use bc_models::BudgetRevisionId;
use bc_models::BudgetWindow;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Period;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::RolloverPolicy;
use bc_models::TagId;
use bc_models::TagPath;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal_macros::dec;

/// Constructs an AUD [`Amount`] from a decimal value.
fn aud(value: Decimal) -> Amount {
    Amount::new(value, CommodityCode::new("AUD"))
}

/// `(code, symbol, name, aliases, decimals, is_iso, symbol_after)`.
type SeedCurrency = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    u8,
    bool,
    bool,
);

/// Returns the first day of the calendar month `months_ago` months before
/// today.
fn month_start(months_ago: i64) -> Date {
    let today = jiff::Zoned::now().date();
    let approx = today.saturating_sub(jiff::Span::new().months(months_ago));
    BudgetWindow::this_month(approx).start
}

/// Returns day `day` (1-based) of the month starting at `start`, clamped to
/// the month's last day so a row never spills into the next month.
fn day_of_month(start: Date, day: i8) -> Date {
    let clamped = day.clamp(1, start.days_in_month());
    start.saturating_add(jiff::Span::new().days(i64::from(clamped) - 1))
}

/// Returns day `day` of the month `months_ago` months before today; see
/// [`day_of_month`].
fn month_day(months_ago: i64, day: i8) -> Date {
    day_of_month(month_start(months_ago), day)
}

/// Constructs the metadata a seeded transaction carries: one `payee` entry.
///
/// `payee` is an ordinary user key holding no privileged position, so a seeded
/// payee is one text entry like any other.
///
/// # Errors
///
/// Returns an error if the literal key `payee` fails validation. `payee` itself
/// is the entry's value and carries no syntax the key rules reach.
fn payee_metadata(payee: &str) -> anyhow::Result<Metadata> {
    Ok(Metadata::new(vec![MetaEntry::new(
        MetaKey::new("payee")?,
        MetaValue::Text(payee.to_owned()),
    )]))
}

/// Constructs a [`Posting`] with a new random ID.
fn posting(account_id: &AccountId, amount: Amount) -> Posting {
    Posting::builder()
        .id(PostingId::new())
        .account_id(account_id.clone())
        .amount(amount)
        .build()
}

/// Constructs a [`Posting`] carrying posting-level tags (e.g. a single
/// reimbursable leg of an otherwise personal transaction).
fn posting_tagged(account_id: &AccountId, amount: Amount, tag_ids: Vec<TagId>) -> Posting {
    Posting::builder()
        .id(PostingId::new())
        .account_id(account_id.clone())
        .amount(amount)
        .tag_ids(tag_ids)
        .build()
}

/// Seeds the hand-written e2e fixture into `pool`.
///
/// # Errors
///
/// Returns an error if any service call fails.
pub async fn seed(pool: &sqlx::SqlitePool) -> anyhow::Result<()> {
    let accounts = AccountService::new(pool.clone());
    let budgets = BudgetService::new(pool.clone());
    let transactions = TransactionService::new(pool.clone());

    // =========================================================================
    // COMMODITIES (9 currencies with richer, collectively-unambiguous aliases)
    // =========================================================================

    let commodities = bc_core::CommodityService::new(pool.clone());
    // Richer-than-default alias sets for demo/testing marker resolution. Codes,
    // symbols, and aliases must stay collectively unambiguous.
    let seed: &[SeedCurrency] = &[
        ("USD", "$", "US Dollar", &["US$", "USD$"], 2, true, false),
        (
            "AUD",
            "A$",
            "Australian Dollar",
            &["AU$", "AUD$"],
            2,
            true,
            false,
        ),
        ("EUR", "€", "Euro", &["EUR€"], 2, true, false),
        ("GBP", "£", "British Pound", &["GBP£"], 2, true, false),
        ("JPY", "¥", "Japanese Yen", &["JP¥"], 0, true, false),
        ("KRW", "₩", "Korean Won", &["KR₩"], 0, true, false),
        ("INR", "₹", "Indian Rupee", &["IN₹"], 2, true, false),
        ("BTC", "₿", "Bitcoin", &["XBT"], 8, false, false),
        ("ETH", "ETH", "Ethereum", &["Ξ"], 9, false, true),
    ];
    for (code, symbol, name, aliases, decimals, is_iso, symbol_after) in seed {
        let c = bc_models::Commodity::builder()
            .code(*code)
            .symbol(*symbol)
            .name(*name)
            .aliases(aliases.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
            .decimals(*decimals)
            .is_iso(*is_iso)
            .symbol_after(*symbol_after)
            .build();
        commodities.create(&c).await?;
    }

    // =========================================================================
    // ACCOUNTS (29 total: 5 root + 24 below them)
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

    // 150 transactions, used by the register lazy-loading E2E spec to force
    // more than one page (`PAGE_SIZE = 100`) without perturbing any other
    // account's transaction count or balance.
    let archive_id = accounts
        .create()
        .name("Archive")
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

    // Dedicated counter account for the Archive paging fixture below, kept
    // separate from `OpeningBalance` so that account's 4 postings / $950.00
    // balance never shifts.
    let archive_opening_id = accounts
        .create()
        .name("ArchiveOpening")
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

    let _drinking_water_id = accounts
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

    let telecommunications_id = accounts
        .create()
        .name("Telecommunications")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .parent_id(&subscriptions_id)
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

    let (groceries_budget, _groceries_rev) = budgets
        .create()
        .account_id(groceries_id.clone())
        .name("Groceries")
        .effective_from(month_start(6))
        .target(aud(dec!(600.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (electricity_budget, _electricity_rev) = budgets
        .create()
        .account_id(electricity_id.clone())
        .name("Electricity")
        .effective_from(month_start(6))
        .target(aud(dec!(350.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (_transport_budget, _transport_rev) = budgets
        .create()
        .account_id(transport_id.clone())
        .name("Transport")
        .effective_from(month_start(6))
        .target(aud(dec!(200.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (_dining_budget, _dining_rev) = budgets
        .create()
        .account_id(dining_id.clone())
        .name("Dining")
        .effective_from(month_start(6))
        .target(aud(dec!(300.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (_entertainment_budget, _entertainment_rev) = budgets
        .create()
        .account_id(entertainment_id.clone())
        .name("Entertainment")
        .effective_from(month_start(6))
        .target(aud(dec!(150.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (_subscriptions_budget, _subscriptions_rev) = budgets
        .create()
        .account_id(subscriptions_id.clone())
        .name("Subscriptions")
        .effective_from(month_start(6))
        .target(aud(dec!(60.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    let (_healthcare_budget, _healthcare_rev) = budgets
        .create()
        .account_id(healthcare_id.clone())
        .name("Healthcare")
        .effective_from(month_start(6))
        .target(aud(dec!(200.00)))
        .period(Period::Monthly)
        .rollover(RolloverPolicy::ResetToZero)
        .intent(BudgetIntent::Limit)
        .call()
        .await?
        .value;

    // =========================================================================
    // REVISIONS (config changes three months ago to demonstrate versioning)
    // =========================================================================

    // Groceries rises from $600 to $700 three months ago.
    budgets
        .revise(
            groceries_budget.id(),
            BudgetRevision::builder()
                .id(BudgetRevisionId::new())
                .budget_id(groceries_budget.id().clone())
                .effective_from(month_start(3))
                .name("Groceries")
                .target(aud(dec!(700.00)))
                .period(Period::Monthly)
                .rollover(RolloverPolicy::ResetToZero)
                .intent(BudgetIntent::Limit)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await?;

    // Electricity drops from $350 to $280 three months ago.
    budgets
        .revise(
            electricity_budget.id(),
            BudgetRevision::builder()
                .id(BudgetRevisionId::new())
                .budget_id(electricity_budget.id().clone())
                .effective_from(month_start(3))
                .name("Electricity")
                .target(aud(dec!(280.00)))
                .period(Period::Monthly)
                .rollover(RolloverPolicy::ResetToZero)
                .intent(BudgetIntent::Limit)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await?;

    // =========================================================================
    // TAGS (a realistic personal-finance taxonomy; some hierarchical)
    // =========================================================================

    let tags = TagService::new(pool.clone());

    // Creates the tag path, returning its leaf id.
    macro_rules! tag {
        ($path:expr) => {
            tags.create_path(&$path.parse::<TagPath>()?).await?
        };
    }

    // Tags attached to transactions/postings below.
    let tag_recurring = tag!("recurring");
    let tag_subscription = tag!("subscription");
    let tag_shared = tag!("shared");
    let tag_commute = tag!("commute");
    let tag_business = tag!("business");
    let tag_reimbursable = tag!("reimbursable");

    // Additional tags rounding out the taxonomy; unattached tags still surface in
    // the tag picker (`list_tags` returns the whole `tags` table).
    for path in [
        "business:travel",
        "business:equipment",
        "tax-deductible",
        "gift",
        "holiday:flights",
        "holiday:lodging",
    ] {
        tags.create_path(&path.parse::<TagPath>()?).await?;
    }

    // =========================================================================
    // TRANSACTIONS (282 total across 6 historical months + current month,
    // including the 150-transaction Archive account and the query fixtures below)
    // =========================================================================

    macro_rules! txn {
        ($date:expr, $payee:expr, $desc:expr, $reconciliation:expr,
         $debit_acct:expr, $debit_amt:expr,
         $credit_acct:expr, $credit_amt:expr) => {
            transactions
                .create(
                    Transaction::builder()
                        .id(TransactionId::new())
                        .date($date)
                        .metadata(payee_metadata($payee)?)
                        .description($desc)
                        .reconciliation($reconciliation)
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

    // Like `txn!`, but attaches transaction-level tags.
    macro_rules! txn_tags {
        ($date:expr, $payee:expr, $desc:expr, $reconciliation:expr,
         $debit_acct:expr, $debit_amt:expr,
         $credit_acct:expr, $credit_amt:expr, $tags:expr) => {
            transactions
                .create(
                    Transaction::builder()
                        .id(TransactionId::new())
                        .date($date)
                        .metadata(payee_metadata($payee)?)
                        .description($desc)
                        .reconciliation($reconciliation)
                        .created_at(Timestamp::now())
                        .tag_ids($tags)
                        .postings(vec![
                            posting($debit_acct, aud($debit_amt)),
                            posting($credit_acct, aud($credit_amt)),
                        ])
                        .build(),
                )
                .await?
        };
    }

    macro_rules! leg {
        ($date:expr, $desc:expr, $acct:expr, $amt:expr) => {
            transactions
                .create(
                    Transaction::builder()
                        .id(TransactionId::new())
                        .date($date)
                        .description($desc)
                        .reconciliation(Reconciliation::Unreconciled)
                        .created_at(Timestamp::now())
                        .postings(vec![posting($acct, aud($amt))])
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
        Reconciliation::Reconciled,
        &checking_id,
        dec!(3500.00),
        &opening_balance_id,
        dec!(-3500.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Savings opening balance",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(8000.00),
        &opening_balance_id,
        dec!(-8000.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Credit card opening balance",
        Reconciliation::Reconciled,
        &opening_balance_id,
        dec!(450.00),
        &credit_card_id,
        dec!(-450.00)
    );
    txn!(
        month_day(6, 1),
        "Opening Balance",
        "Car loan opening balance",
        Reconciliation::Reconciled,
        &opening_balance_id,
        dec!(12000.00),
        &car_loan_id,
        dec!(-12000.00)
    );

    // -------------------------------------------------------------------------
    // 6 months ago
    // -------------------------------------------------------------------------

    txn_tags!(
        month_day(6, 5),
        "Client A",
        "November freelance payment",
        Reconciliation::Reconciled,
        &checking_id,
        dec!(800.00),
        &freelance_id,
        dec!(-800.00),
        vec![tag_business.clone()]
    );
    txn!(
        month_day(6, 15),
        "Employer Ltd",
        "November paycheck",
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(6, 20),
        "Visa",
        "November credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(6, 25),
        "Transfer",
        "November savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(6, 30),
        "Bank",
        "November savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(9.50),
        &interest_id,
        dec!(-9.50)
    );
    txn!(
        month_day(6, 1),
        "Car Finance",
        "November car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(6, 3),
        "Woolworths",
        "November groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(6, 14),
        "Coles",
        "November fortnightly groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(6, 22),
        "IGA",
        "November grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    // A shared meal (transaction-level `shared`) whose dining leg is also
    // `reimbursable` (posting-level) — exercises both tag scopes.
    transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(6, 8))
                .metadata(payee_metadata("The Local Bistro")?)
                .description("November dinner")
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .tag_ids(vec![tag_shared.clone()])
                .postings(vec![
                    posting_tagged(&dining_id, aud(dec!(85.00)), vec![tag_reimbursable.clone()]),
                    posting(&credit_card_id, aud(dec!(-85.00))),
                ])
                .build(),
        )
        .await?;
    txn!(
        month_day(6, 20),
        "The Coffee Club",
        "November coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(6, 12),
        "AGL Energy",
        "November electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn_tags!(
        month_day(6, 12),
        "Telstra",
        "November internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00),
        vec![tag_recurring.clone()]
    );
    txn!(
        month_day(6, 28),
        "Origin Energy",
        "November gas bill",
        Reconciliation::Reconciled,
        &sewer_id,
        dec!(130.00),
        &checking_id,
        dec!(-130.00)
    );
    txn_tags!(
        month_day(6, 18),
        "Opal Card",
        "November transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00),
        vec![tag_commute.clone()]
    );
    txn_tags!(
        month_day(6, 3),
        "Netflix",
        "November streaming subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99),
        vec![tag_recurring.clone(), tag_subscription.clone()]
    );
    txn!(
        month_day(6, 3),
        "Spotify",
        "November music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(6, 10),
        "iCloud",
        "November cloud storage",
        Reconciliation::Reconciled,
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
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(5, 20),
        "Visa",
        "December credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(5, 25),
        "Transfer",
        "December savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(5, 31),
        "Bank",
        "December savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(10.00),
        &interest_id,
        dec!(-10.00)
    );
    txn!(
        month_day(5, 1),
        "Car Finance",
        "December car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(5, 3),
        "Woolworths",
        "December groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(5, 14),
        "Coles",
        "December fortnightly groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(5, 22),
        "IGA",
        "December grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(5, 8),
        "The Local Bistro",
        "December dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(5, 20),
        "The Coffee Club",
        "December coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(5, 22),
        "Fine Dining Co",
        "Christmas dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(210.00),
        &credit_card_id,
        dec!(-210.00)
    );
    txn!(
        month_day(5, 31),
        "NYE Restaurant",
        "New Year's Eve dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(175.00),
        &credit_card_id,
        dec!(-175.00)
    );
    txn!(
        month_day(5, 12),
        "AGL Energy",
        "December electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(5, 12),
        "Telstra",
        "December internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(5, 18),
        "Opal Card",
        "December transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(5, 3),
        "Netflix",
        "December streaming subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(5, 3),
        "Spotify",
        "December music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(5, 10),
        "iCloud",
        "December cloud storage",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(5, 15),
        "Event Cinemas",
        "December cinema",
        Reconciliation::Reconciled,
        &entertainment_id,
        dec!(45.00),
        &credit_card_id,
        dec!(-45.00)
    );
    txn!(
        month_day(5, 20),
        "Live Nation",
        "December concert",
        Reconciliation::Reconciled,
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
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(4, 20),
        "Visa",
        "January credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(4, 25),
        "Transfer",
        "January savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(4, 31),
        "Bank",
        "January savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(10.50),
        &interest_id,
        dec!(-10.50)
    );
    txn!(
        month_day(4, 1),
        "Car Finance",
        "January car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(4, 3),
        "Woolworths",
        "January groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(190.00),
        &credit_card_id,
        dec!(-190.00)
    );
    txn!(
        month_day(4, 14),
        "Coles",
        "January fortnightly groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(4, 22),
        "IGA",
        "January grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(4, 28),
        "Harris Farm",
        "January organic groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(80.00),
        &credit_card_id,
        dec!(-80.00)
    );
    txn!(
        month_day(4, 8),
        "The Local Bistro",
        "January dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(4, 20),
        "The Coffee Club",
        "January coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(4, 12),
        "AGL Energy",
        "January electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(4, 12),
        "Telstra",
        "January internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(4, 18),
        "Opal Card",
        "January transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(4, 3),
        "Netflix",
        "January streaming subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(4, 3),
        "Spotify",
        "January music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(4, 10),
        "iCloud",
        "January cloud storage",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(4, 10),
        "City Medical Centre",
        "January GP visit",
        Reconciliation::Reconciled,
        &healthcare_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(4, 11),
        "Chemist Warehouse",
        "January pharmacy",
        Reconciliation::Reconciled,
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
                .metadata(payee_metadata("Employer Ltd")?)
                .description("January paycheck — duplicate (to be voided)")
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&checking_id, aud(dec!(5200.00))),
                    posting(&salary_id, aud(dec!(-5200.00))),
                ])
                .build(),
        )
        .await?
        .into_inner();
    drop(transactions.reverse(&voided_jan_paycheck).await?);

    // -------------------------------------------------------------------------
    // 3 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(3, 15),
        "Employer Ltd",
        "February paycheck",
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(3, 20),
        "Visa",
        "February credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(3, 25),
        "Transfer",
        "February savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(3, 28),
        "Bank",
        "February savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(11.00),
        &interest_id,
        dec!(-11.00)
    );
    txn!(
        month_day(3, 1),
        "Car Finance",
        "February car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(3, 3),
        "Woolworths",
        "February groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(3, 14),
        "Coles",
        "February fortnightly groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(3, 22),
        "IGA",
        "February grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(3, 8),
        "The Local Bistro",
        "February dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(3, 20),
        "The Coffee Club",
        "February coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(3, 12),
        "AGL Energy",
        "February electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(3, 12),
        "Telstra",
        "February internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(3, 5),
        "BP Service Station",
        "February petrol",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(3, 18),
        "Opal Card",
        "February transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(3, 3),
        "Netflix",
        "February streaming subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(3, 3),
        "Spotify",
        "February music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(3, 10),
        "iCloud",
        "February cloud storage",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(4.49),
        &credit_card_id,
        dec!(-4.49)
    );
    txn!(
        month_day(3, 20),
        "Client B",
        "February freelance payment",
        Reconciliation::Reconciled,
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
                .metadata(payee_metadata("Woolworths")?)
                .description("February groceries — duplicate (to be voided)")
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&groceries_id, aud(dec!(140.00))),
                    posting(&credit_card_id, aud(dec!(-140.00))),
                ])
                .build(),
        )
        .await?
        .into_inner();
    drop(transactions.reverse(&voided_feb_woolworths).await?);

    // A three-way split: one purchase across two categories.
    // accounts-posting-mid-delete.spec.ts finds it by its unique payee and
    // deletes the middle leg (#210). Insertion order is display order.
    transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(3, 12))
                .metadata(payee_metadata("Costco")?)
                .description("Bulk shop")
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&groceries_id, aud(dec!(84.00))),
                    posting(&healthcare_id, aud(dec!(26.50))),
                    posting(&checking_id, aud(dec!(-110.50))),
                ])
                .build(),
        )
        .await?;

    // -------------------------------------------------------------------------
    // 2 months ago
    // -------------------------------------------------------------------------

    txn!(
        month_day(2, 15),
        "Employer Ltd",
        "March paycheck",
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(2, 20),
        "Visa",
        "March credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(2, 25),
        "Transfer",
        "March savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1200.00),
        &checking_id,
        dec!(-1200.00)
    );
    txn!(
        month_day(2, 31),
        "Bank",
        "March savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(11.50),
        &interest_id,
        dec!(-11.50)
    );
    txn!(
        month_day(2, 1),
        "Car Finance",
        "March car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(2, 3),
        "Woolworths",
        "March groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(2, 14),
        "Coles",
        "March fortnightly groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(2, 22),
        "IGA",
        "March grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(2, 8),
        "The Local Bistro",
        "March dinner",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(2, 20),
        "The Coffee Club",
        "March coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(2, 12),
        "AGL Energy",
        "March electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(2, 12),
        "Telstra",
        "March internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(2, 28),
        "Origin Energy",
        "March gas bill",
        Reconciliation::Reconciled,
        &sewer_id,
        dec!(130.00),
        &checking_id,
        dec!(-130.00)
    );
    txn!(
        month_day(2, 18),
        "Opal Card",
        "March transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(2, 3),
        "Netflix",
        "March streaming subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(2, 3),
        "Spotify",
        "March music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(2, 10),
        "iCloud",
        "March cloud storage",
        Reconciliation::Reconciled,
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
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200.00),
        &salary_id,
        dec!(-5200.00)
    );
    txn!(
        month_day(1, 20),
        "Visa",
        "April credit card payment",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(800.00),
        &checking_id,
        dec!(-800.00)
    );
    txn!(
        month_day(1, 25),
        "Transfer",
        "April savings transfer",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(1000.00),
        &checking_id,
        dec!(-1000.00)
    );
    txn!(
        month_day(1, 30),
        "Bank",
        "April savings interest",
        Reconciliation::Reconciled,
        &savings_id,
        dec!(12.00),
        &interest_id,
        dec!(-12.00)
    );
    txn!(
        month_day(1, 1),
        "Car Finance",
        "April car loan repayment",
        Reconciliation::Reconciled,
        &car_loan_id,
        dec!(350.00),
        &checking_id,
        dec!(-350.00)
    );
    txn!(
        month_day(1, 3),
        "Woolworths",
        "April groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(140.00),
        &credit_card_id,
        dec!(-140.00)
    );
    txn!(
        month_day(1, 14),
        "Coles",
        "April fortnightly groceries",
        Reconciliation::Unreconciled,
        &groceries_id,
        dec!(110.00),
        &credit_card_id,
        dec!(-110.00)
    );
    txn!(
        month_day(1, 22),
        "IGA",
        "April grocery top-up",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(55.00),
        &credit_card_id,
        dec!(-55.00)
    );
    txn!(
        month_day(1, 8),
        "The Local Bistro",
        "April dinner",
        Reconciliation::Unreconciled,
        &dining_id,
        dec!(85.00),
        &credit_card_id,
        dec!(-85.00)
    );
    txn!(
        month_day(1, 20),
        "The Coffee Club",
        "April coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(18.50),
        &credit_card_id,
        dec!(-18.50)
    );
    txn!(
        month_day(1, 12),
        "AGL Energy",
        "April electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(210.00),
        &checking_id,
        dec!(-210.00)
    );
    txn!(
        month_day(1, 12),
        "Telstra",
        "April internet bill",
        Reconciliation::Reconciled,
        &telecommunications_id,
        dec!(89.00),
        &checking_id,
        dec!(-89.00)
    );
    txn!(
        month_day(1, 18),
        "Opal Card",
        "April transit top-up",
        Reconciliation::Reconciled,
        &transport_id,
        dec!(50.00),
        &checking_id,
        dec!(-50.00)
    );
    txn!(
        month_day(1, 3),
        "Netflix",
        "April streaming subscription",
        Reconciliation::Unreconciled,
        &subscriptions_id,
        dec!(22.99),
        &credit_card_id,
        dec!(-22.99)
    );
    txn!(
        month_day(1, 3),
        "Spotify",
        "April music subscription",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(12.99),
        &credit_card_id,
        dec!(-12.99)
    );
    txn!(
        month_day(1, 10),
        "iCloud",
        "April cloud storage",
        Reconciliation::Reconciled,
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
        Reconciliation::Reconciled,
        &checking_id,
        dec!(5200),
        &salary_id,
        dec!(-5200)
    );
    txn!(
        month_day(0, 3),
        "Supermarket",
        "Groceries",
        Reconciliation::Reconciled,
        &groceries_id,
        dec!(95),
        &checking_id,
        dec!(-95)
    );
    txn!(
        month_day(0, 4),
        "The Coffee Club",
        "Coffee",
        Reconciliation::Reconciled,
        &dining_id,
        dec!(6.50),
        &credit_card_id,
        dec!(-6.50)
    );
    txn!(
        month_day(0, 2),
        "Power Company",
        "Electricity bill",
        Reconciliation::Reconciled,
        &electricity_id,
        dec!(120),
        &checking_id,
        dec!(-120)
    );
    // Current-month `recurring`-tagged CreditCard activity: a membership charge
    // (outflow) and a refund (inflow) so a `tag:recurring` filter on CreditCard
    // resolves to in-window flows on both sides, not just a pre-window opening.
    txn_tags!(
        month_day(0, 6),
        "Gym Membership",
        "Monthly membership",
        Reconciliation::Reconciled,
        &subscriptions_id,
        dec!(80.00),
        &credit_card_id,
        dec!(-80.00),
        vec![tag_recurring.clone()]
    );
    txn_tags!(
        month_day(0, 8),
        "Subscription Refund",
        "Overcharge refund",
        Reconciliation::Reconciled,
        &credit_card_id,
        dec!(15.00),
        &subscriptions_id,
        dec!(-15.00),
        vec![tag_recurring.clone()]
    );

    // -------------------------------------------------------------------------
    // Archive account (150 transactions) — exists purely to force the
    // register past `PAGE_SIZE` for the lazy-loading E2E spec. Postings are
    // balanced against `ArchiveOpening` (a dedicated equity account, not
    // `OpeningBalance`), so no other account's transaction count or balance
    // shifts. Spread across the 7 seeded months (day 1-28, safe for every
    // month length) rather than piled on one date.
    // -------------------------------------------------------------------------
    let mut archive_count = 0_u32;
    let mut archive_amount_is_small = true;
    'archive: for months_ago in (0_i64..=6_i64).rev() {
        for day in 1_i8..=28_i8 {
            if archive_count >= 150_u32 {
                break 'archive;
            }
            archive_count = archive_count.saturating_add(1);
            let amount = if archive_amount_is_small {
                dec!(10.00)
            } else {
                dec!(25.00)
            };
            archive_amount_is_small = !archive_amount_is_small;
            txn!(
                month_day(months_ago, day),
                "Archive Co",
                format!("Archive deposit {archive_count:03}"),
                Reconciliation::Reconciled,
                &archive_id,
                amount,
                &archive_opening_id,
                -amount
            );
        }
    }

    // -------------------------------------------------------------------------
    // Unmerged transfer legs (interim single-posting imports) — three pairs the
    // review UI can suggest. Distinct magnitudes prevent cross-pairing.
    // -------------------------------------------------------------------------
    leg!(
        month_day(0, 5),
        "TFR TO CAR LOAN",
        &savings_id,
        dec!(-500.00)
    );
    leg!(
        month_day(0, 6),
        "LOAN REPAYMENT",
        &car_loan_id,
        dec!(500.00)
    );

    leg!(
        month_day(0, 7),
        "TFR TO SAVINGS",
        &checking_id,
        dec!(-250.00)
    );
    leg!(month_day(0, 8), "DEPOSIT", &savings_id, dec!(250.00));

    leg!(month_day(0, 9), "CARD PAYMENT", &savings_id, dec!(-1000.00));
    leg!(
        month_day(0, 10),
        "PAYMENT RECEIVED",
        &credit_card_id,
        dec!(1000.00)
    );

    // =========================================================================
    // QUERY LANGUAGE FIXTURES: the split card payment of the query spec's §2,
    // typed metadata keys, a repeated key and a mismatched value. Dedicated
    // accounts keep every other register and budget unchanged.
    // =========================================================================

    let split_card_id = accounts
        .create()
        .name("SplitCard")
        .account_type(AccountType::Liability)
        .kind(AccountKind::DepositAccount)
        .parent_id(&liabilities_id)
        .call()
        .await?;
    let split_id = accounts
        .create()
        .name("Split")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .parent_id(&assets_id)
        .call()
        .await?;
    let mut split_legs = Vec::new();
    for name in ["Me", "Partner", "Holiday", "Shared"] {
        split_legs.push(
            accounts
                .create()
                .name(name)
                .account_type(AccountType::Asset)
                .kind(AccountKind::DepositAccount)
                .parent_id(&split_id)
                .call()
                .await?,
        );
    }
    let [
        split_me_id,
        split_partner_id,
        split_holiday_id,
        split_shared_id,
    ] = <[AccountId; 4]>::try_from(split_legs)
        .map_err(|_legs| anyhow::anyhow!("expected four split accounts"))?;
    let tag_me = tag!("me");
    let tag_partner = tag!("partner");
    let tag_flights = tags
        .find_by_path(&"holiday:flights".parse::<TagPath>()?)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the holiday:flights tag is created above"))?;

    let entry = |key: &str, value: MetaValue| -> anyhow::Result<MetaEntry> {
        Ok(MetaEntry::new(MetaKey::new(key)?, value))
    };

    // One card payment split four ways, with a leg tag on each personal share.
    transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(2, 14))
                .metadata(Metadata::new(vec![
                    entry("payee", MetaValue::Text("Example Travel Agency".to_owned()))?,
                    entry("receipt", MetaValue::Text("R-1001".to_owned()))?,
                    entry("receipt", MetaValue::Text("R-1002".to_owned()))?,
                ]))
                .description("Shared holiday booking")
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&split_card_id, aud(dec!(5000.00))),
                    posting_tagged(&split_me_id, aud(dec!(-1000.00)), vec![tag_me]),
                    posting_tagged(&split_partner_id, aud(dec!(-1000.00)), vec![tag_partner]),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(split_holiday_id.clone())
                        .amount(aud(dec!(-2000.00)))
                        .tag_ids(vec![tag_flights])
                        .metadata(Metadata::new(vec![entry(
                            "deposit",
                            MetaValue::Amount(aud(dec!(500.00))),
                        )?]))
                        .build(),
                    posting(&split_shared_id, aud(dec!(-1000.00))),
                ])
                .build(),
        )
        .await?;
    // A number and a date key; registers `odometer` as a number.
    transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(2, 20))
                .metadata(Metadata::new(vec![
                    entry("payee", MetaValue::Text("Example Fuel Stop".to_owned()))?,
                    entry("odometer", MetaValue::Number(dec!(48210)))?,
                    entry("due", MetaValue::Date(month_day(1, 28)))?,
                ]))
                .description("Road trip fuel")
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&split_holiday_id, aud(dec!(80.00))),
                    posting(&split_card_id, aud(dec!(-80.00))),
                ])
                .build(),
        )
        .await?;
    // Text under the number key `odometer`: stored flagged as mismatched.
    transactions
        .create(
            Transaction::builder()
                .id(TransactionId::new())
                .date(month_day(2, 27))
                .metadata(Metadata::new(vec![
                    entry("payee", MetaValue::Text("Example Fuel Stop".to_owned()))?,
                    entry("odometer", MetaValue::Text("not recorded".to_owned()))?,
                ]))
                .description("Road trip fuel top-up")
                .reconciliation(Reconciliation::Unreconciled)
                .created_at(Timestamp::now())
                .postings(vec![
                    posting(&split_holiday_id, aud(dec!(40.00))),
                    posting(&split_card_id, aud(dec!(-40.00))),
                ])
                .build(),
        )
        .await?;

    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::fmt::Write as _;
    use std::str::FromStr as _;

    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

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

    #[rstest]
    #[case::short_february(date(2026, 2, 1), 30, date(2026, 2, 28))]
    #[case::leap_february(date(2028, 2, 1), 31, date(2028, 2, 29))]
    #[case::thirty_day_month(date(2026, 4, 1), 31, date(2026, 4, 30))]
    #[case::full_month(date(2026, 1, 1), 31, date(2026, 1, 31))]
    #[case::mid_month(date(2026, 4, 1), 15, date(2026, 4, 15))]
    fn day_of_month_clamps(#[case] start: Date, #[case] day: i8, #[case] expected: Date) {
        assert_eq!(day_of_month(start, day), expected);
    }

    #[test]
    fn aud_constructs_correct_amount() {
        assert_eq!(
            aud(dec!(42.50)),
            Amount::new(dec!(42.50), CommodityCode::new("AUD"))
        );
    }

    /// Seeds the fixture into a fresh database under `dir`.
    async fn seeded_pool(dir: &tempfile::TempDir) -> sqlx::SqlitePool {
        let pool = bc_core::open_db_at(&dir.path().join("fixture.db"))
            .await
            .expect("open temp db");
        seed(&pool).await.expect("seed the fixture");
        pool
    }

    /// Renders the seeded facts the e2e specs rely on.
    ///
    /// Holds no absolute dates, so it does not change with the day it runs.
    async fn inventory(pool: &sqlx::SqlitePool) -> String {
        let mut out = String::new();

        // MARK: Accounts
        let accounts: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, name, parent_id FROM accounts")
                .fetch_all(pool)
                .await
                .expect("read accounts");
        let by_id: HashMap<&str, (&str, Option<&str>)> = accounts
            .iter()
            .map(|(id, name, parent)| (id.as_str(), (name.as_str(), parent.as_deref())))
            .collect();
        let path_of = |id: &str| -> String {
            let mut parts = Vec::new();
            let mut cursor = Some(id);
            while let Some(current) = cursor {
                let (name, parent) = *by_id.get(current).expect("known account");
                parts.push(name);
                cursor = parent;
            }
            parts.reverse();
            parts.join(":")
        };
        let touched: HashMap<String, i64> = sqlx::query_as(
            "SELECT account_id, COUNT(DISTINCT transaction_id) FROM postings GROUP BY account_id",
        )
        .fetch_all(pool)
        .await
        .expect("count transactions per account")
        .into_iter()
        .collect();
        let mut paths: Vec<(String, i64)> = accounts
            .iter()
            .map(|(id, _, _)| (path_of(id), touched.get(id).copied().unwrap_or(0)))
            .collect();
        paths.sort();
        writeln!(out, "# transactions per account (direct postings)").expect("write");
        for (path, count) in paths {
            writeln!(out, "{path}: {count}").expect("write");
        }

        // MARK: Posting counts
        let histogram: Vec<(i64, i64)> = sqlx::query_as(
            "SELECT n, COUNT(*) FROM \
               (SELECT COUNT(*) AS n FROM postings GROUP BY transaction_id) \
             GROUP BY n ORDER BY n",
        )
        .fetch_all(pool)
        .await
        .expect("posting histogram");
        writeln!(out, "\n# transactions by posting count").expect("write");
        for (postings, transactions) in histogram {
            writeln!(out, "{postings}: {transactions}").expect("write");
        }

        // MARK: Balance
        // Amounts are TEXT, so the sums stay in Rust.
        let legs: Vec<(String, Option<String>, Option<String>)> =
            sqlx::query_as("SELECT transaction_id, amount, commodity FROM postings")
                .fetch_all(pool)
                .await
                .expect("read postings");
        let mut sums: BTreeMap<(String, String), Decimal> = BTreeMap::new();
        let mut elided: BTreeMap<String, bool> = BTreeMap::new();
        for (tx, raw_amount, raw_commodity) in legs {
            let has_elided = elided.entry(tx.clone()).or_insert(false);
            match (raw_amount, raw_commodity) {
                (Some(amount), Some(commodity)) => {
                    *sums.entry((tx, commodity)).or_default() +=
                        Decimal::from_str(&amount).expect("decimal amount");
                }
                _ => *has_elided = true,
            }
        }
        let mut unbalanced: Vec<&str> = sums
            .iter()
            .filter(|((tx, _), sum)| {
                !sum.is_zero() && !elided.get(tx.as_str()).copied().unwrap_or(false)
            })
            .map(|((tx, _), _)| tx.as_str())
            .collect();
        unbalanced.dedup();
        writeln!(out, "\n# unbalanced transactions: {}", unbalanced.len()).expect("write");

        // MARK: Payees
        let payees: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT value_text FROM transaction_metadata \
             WHERE key = 'payee' ORDER BY value_text",
        )
        .fetch_all(pool)
        .await
        .expect("read payees");
        writeln!(out, "\n# payees").expect("write");
        for (payee,) in payees {
            writeln!(out, "{payee}").expect("write");
        }

        // MARK: Budgets
        // Revision dates as month offsets from this month, so the snapshot
        // does not move with the clock.
        let this_month = month_start(0);
        let revisions: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT r.budget_id, \
                    (SELECT name FROM budget_revisions f WHERE f.budget_id = r.budget_id \
                      ORDER BY f.effective_from LIMIT 1), \
                    r.effective_from \
               FROM budget_revisions r ORDER BY r.effective_from",
        )
        .fetch_all(pool)
        .await
        .expect("read budget revisions");
        let mut budgets: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (_, name, effective_from) in revisions {
            let date = Date::from_str(&effective_from).expect("revision date");
            let months = (i32::from(date.year()) * 12_i32 + i32::from(date.month()))
                - (i32::from(this_month.year()) * 12_i32 + i32::from(this_month.month()));
            let offset = if date.day() == 1 {
                format!("{months:+}")
            } else {
                format!("{months:+} day {}", date.day())
            };
            budgets.entry(name).or_default().push(offset);
        }
        writeln!(out, "\n# budget revisions (months from this month)").expect("write");
        for (name, offsets) in budgets {
            writeln!(out, "{name}: {}", offsets.join(", ")).expect("write");
        }

        out
    }

    /// Stack for the thread that polls the seed future. The fixture is one
    /// large async fn, and in a debug build its poll frame overflows the
    /// default 2 MiB test-thread stack.
    const SEED_STACK_BYTES: usize = 64 * 1024 * 1024;

    /// Seeds a fresh database on a big-stack thread and runs `f` against its
    /// pool there, returning the result.
    fn with_seeded_pool<F, Fut, T>(f: F) -> T
    where
        F: FnOnce(sqlx::SqlitePool) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = T>,
        T: Send + 'static,
    {
        std::thread::Builder::new()
            .stack_size(SEED_STACK_BYTES)
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("build runtime")
                    .block_on(async {
                        let dir = tempfile::tempdir().expect("tempdir");
                        let pool = seeded_pool(&dir).await;
                        let out = f(pool.clone()).await;
                        pool.close().await;
                        out
                    })
            })
            .expect("spawn seeding thread")
            .join()
            .expect("seeding thread panicked")
    }

    #[test]
    fn fixture_inventory() {
        let rendered = with_seeded_pool(|pool| async move { inventory(&pool).await });
        insta::assert_snapshot!(rendered);
    }

    #[test]
    fn the_split_is_the_only_costco_transaction_and_has_three_postings() {
        let postings: Vec<(i64,)> = with_seeded_pool(|pool| async move {
            sqlx::query_as(
                "SELECT COUNT(p.id) FROM transaction_metadata m \
                   JOIN postings p ON p.transaction_id = m.transaction_id \
                  WHERE m.key = 'payee' AND m.value_text = 'Costco' \
                  GROUP BY m.transaction_id",
            )
            .fetch_all(&pool)
            .await
            .expect("query Costco transactions")
        });
        assert_eq!(
            postings,
            vec![(3,)],
            "accounts-posting-mid-delete.spec.ts finds it by payee"
        );
    }
}
