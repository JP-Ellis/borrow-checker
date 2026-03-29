//! Integration tests: run importer against fixture Ledger files.

use bc_core::ImportConfig;
use bc_core::Importer as _;
use bc_format_ledger::LedgerImporter;
use jiff::civil::date;
use pretty_assertions::assert_eq;

#[test]
#[expect(
    clippy::tests_outside_test_module,
    reason = "integration tests live in tests/ by convention"
)]
#[expect(
    clippy::indexing_slicing,
    reason = "test indices are known to be valid"
)]
fn basic_two_transactions() {
    let bytes = include_bytes!("fixtures/basic.ledger");
    let txs = LedgerImporter::new()
        .import(bytes, &ImportConfig::default())
        .expect("import");
    assert_eq!(txs.len(), 2);
    assert_eq!(txs[0].date, date(2025, 1, 15));
    assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
    assert_eq!(txs[1].payee.as_deref(), Some("Salary"));
}

#[test]
#[expect(
    clippy::tests_outside_test_module,
    reason = "integration tests live in tests/ by convention"
)]
fn elided_amount_produces_one_raw_transaction() {
    let bytes = include_bytes!("fixtures/elided_amount.ledger");
    let txs = LedgerImporter::new()
        .import(bytes, &ImportConfig::default())
        .expect("import");
    assert_eq!(txs.len(), 1);
}
