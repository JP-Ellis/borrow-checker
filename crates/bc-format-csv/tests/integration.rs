//! Integration tests for the CSV importer.

// Integration test files compile as a separate crate; #[test] functions are
// always outside a #[cfg(test)] module, which clippy flags.
#![expect(
    clippy::tests_outside_test_module,
    reason = "integration tests live in tests/ and are compiled as a standalone binary"
)]
// Tests legitimately use `.expect()` to panic on unexpected failures.
#![expect(
    clippy::expect_used,
    reason = "panicking on unexpected failure is appropriate in tests"
)]

use bc_core::ImportConfig;
use bc_core::Importer as _;
use bc_format_csv::AmountColumns;
use bc_format_csv::CsvConfig;
use bc_format_csv::CsvImporter;
use bc_format_csv::Preamble;
use bc_models::Amount;
use bc_models::CommodityCode;
use jiff::civil::date;
use pretty_assertions::assert_eq;
use rust_decimal_macros::dec;

/// Builds an [`ImportConfig`] from a [`CsvConfig`], panicking on serialisation
/// failure (acceptable in tests).
fn make_config(cfg: &CsvConfig) -> ImportConfig {
    ImportConfig::from_typed(cfg).expect("config serialisation should succeed")
}

// ─── Test 1: simple.csv ──────────────────────────────────────────────────────

/// Imports a simple CSV with a single signed `Amount` column and verifies that
/// all three rows are parsed correctly with the right dates, descriptions, and
/// signed decimal values.
#[test]
fn simple_csv_three_rows_correct_values() {
    let bytes = include_bytes!("fixtures/simple.csv");
    let cfg = make_config(
        &CsvConfig::builder()
            .description_column("Description".to_owned())
            .commodity("AUD".to_owned())
            .build(),
    );

    let importer = CsvImporter::new();
    let txns = importer.import(bytes, &cfg).expect("import should succeed");

    assert_eq!(txns.len(), 3);

    let mut iter = txns.iter();

    let tx0 = iter.next().expect("first transaction should exist");
    assert_eq!(tx0.date, date(2025, 1, 15));
    assert_eq!(
        tx0.amount,
        Amount::new(dec!(-50.00), CommodityCode::new("AUD"))
    );
    assert_eq!(tx0.description, "Woolworths");

    let tx1 = iter.next().expect("second transaction should exist");
    assert_eq!(tx1.date, date(2025, 1, 16));
    assert_eq!(
        tx1.amount,
        Amount::new(dec!(3000.00), CommodityCode::new("AUD"))
    );
    assert_eq!(tx1.description, "Salary");

    let tx2 = iter.next().expect("third transaction should exist");
    assert_eq!(tx2.date, date(2025, 1, 17));
    assert_eq!(
        tx2.amount,
        Amount::new(dec!(-1500.00), CommodityCode::new("AUD"))
    );
    assert_eq!(tx2.description, "Rent");
}

// ─── Test 2: bank_preamble.csv with AutoDetect ───────────────────────────────

/// Imports a bank CSV with four metadata preamble lines followed by a blank
/// line, using `AutoDetect` to find the header. Verifies that debit rows produce
/// negative amounts and credit rows produce positive amounts, and that the
/// running balance is populated.
#[test]
fn bank_preamble_auto_detect_two_rows_correct_signs_and_balance() {
    let bytes = include_bytes!("fixtures/bank_preamble.csv");
    let cfg = make_config(
        &CsvConfig::builder()
            .preamble(Preamble::AutoDetect { max_scan_lines: 20 })
            .date_format("%d/%m/%Y".to_owned())
            .description_column("Description".to_owned())
            .amount_columns(AmountColumns::SplitDebitCredit {
                debit_column: "Debit".into(),
                credit_column: "Credit".into(),
            })
            .balance_column("Balance".to_owned())
            .commodity("AUD".to_owned())
            .build(),
    );

    let importer = CsvImporter::new();
    let txns = importer.import(bytes, &cfg).expect("import should succeed");

    assert_eq!(txns.len(), 2);

    let mut iter = txns.iter();

    // Woolworths: debit 50.00 → negative amount
    let tx0 = iter.next().expect("first transaction should exist");
    assert_eq!(tx0.date, date(2025, 1, 15));
    assert_eq!(
        tx0.amount,
        Amount::new(dec!(-50.00), CommodityCode::new("AUD"))
    );
    assert_eq!(
        tx0.balance,
        Some(Amount::new(dec!(4950.00), CommodityCode::new("AUD")))
    );

    // Salary: credit 3000.00 → positive amount
    let tx1 = iter.next().expect("second transaction should exist");
    assert_eq!(tx1.date, date(2025, 1, 16));
    assert_eq!(
        tx1.amount,
        Amount::new(dec!(3000.00), CommodityCode::new("AUD"))
    );
    assert_eq!(
        tx1.balance,
        Some(Amount::new(dec!(7950.00), CommodityCode::new("AUD")))
    );
}

// ─── Test 3: bank_preamble.csv with SkipLines ────────────────────────────────

/// Same fixture as test 2, but uses `SkipLines { lines: 5 }` to skip the four
/// metadata rows and the blank line. Verifies the same result.
#[test]
fn bank_preamble_skip_lines_same_result_as_auto_detect() {
    let bytes = include_bytes!("fixtures/bank_preamble.csv");
    let cfg = make_config(
        &CsvConfig::builder()
            .preamble(Preamble::SkipLines { lines: 5 })
            .date_format("%d/%m/%Y".to_owned())
            .description_column("Description".to_owned())
            .amount_columns(AmountColumns::SplitDebitCredit {
                debit_column: "Debit".into(),
                credit_column: "Credit".into(),
            })
            .balance_column("Balance".to_owned())
            .commodity("AUD".to_owned())
            .build(),
    );

    let importer = CsvImporter::new();
    let txns = importer.import(bytes, &cfg).expect("import should succeed");

    assert_eq!(txns.len(), 2);

    let mut iter = txns.iter();

    let tx0 = iter.next().expect("first transaction should exist");
    assert_eq!(tx0.date, date(2025, 1, 15));
    assert_eq!(
        tx0.amount,
        Amount::new(dec!(-50.00), CommodityCode::new("AUD"))
    );

    let tx1 = iter.next().expect("second transaction should exist");
    assert_eq!(tx1.date, date(2025, 1, 16));
    assert_eq!(
        tx1.amount,
        Amount::new(dec!(3000.00), CommodityCode::new("AUD"))
    );
}

// ─── Test 4: debit_credit.csv — payee and description columns ────────────────

/// Imports a CSV with separate `Payee` and `Narration` columns, verifying that
/// both are populated on the resulting transactions.
#[test]
fn debit_credit_csv_payee_and_description_populated() {
    let bytes = include_bytes!("fixtures/debit_credit.csv");
    let cfg = make_config(
        &CsvConfig::builder()
            .amount_columns(AmountColumns::SplitDebitCredit {
                debit_column: "Debit".into(),
                credit_column: "Credit".into(),
            })
            .payee_column("Payee".to_owned())
            .description_column("Narration".to_owned())
            .balance_column("Balance".to_owned())
            .commodity("AUD".to_owned())
            .build(),
    );

    let importer = CsvImporter::new();
    let txns = importer.import(bytes, &cfg).expect("import should succeed");

    assert_eq!(txns.len(), 3);

    let mut iter = txns.iter();

    let tx0 = iter.next().expect("first transaction should exist");
    assert_eq!(tx0.payee.as_deref(), Some("Woolworths"));
    assert_eq!(tx0.description, "Weekly groceries");
    assert_eq!(
        tx0.amount,
        Amount::new(dec!(-50.00), CommodityCode::new("AUD"))
    );

    let tx1 = iter.next().expect("second transaction should exist");
    assert_eq!(tx1.payee.as_deref(), Some("Employer"));
    assert_eq!(tx1.description, "Monthly salary");
    assert_eq!(
        tx1.amount,
        Amount::new(dec!(3000.00), CommodityCode::new("AUD"))
    );

    let tx2 = iter.next().expect("third transaction should exist");
    assert_eq!(tx2.payee.as_deref(), Some("Landlord"));
    assert_eq!(tx2.description, "January rent");
    assert_eq!(
        tx2.amount,
        Amount::new(dec!(-1500.00), CommodityCode::new("AUD"))
    );
}
