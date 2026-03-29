//! Integration tests: run importer against fixture Beancount files.

#[cfg(test)]
mod tests {
    use bc_core::ImportConfig;
    use bc_core::Importer as _;
    use bc_format_beancount::BeancountImporter;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    #[test]
    fn basic_two_transactions() {
        let bytes = include_bytes!("../tests/fixtures/basic.beancount");
        let txs = BeancountImporter::new()
            .import(bytes, &ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 2);
        let first = txs.first().expect("should have first transaction");
        let second = txs.get(1).expect("should have second transaction");
        assert_eq!(first.date, date(2025, 1, 15));
        assert_eq!(first.payee.as_deref(), Some("Woolworths"));
        assert_eq!(first.description, "Weekly groceries");
        assert_eq!(second.payee.as_deref(), Some("Employer"));
    }
}
