//! Integration tests: run importer against fixture OFX files.

#[cfg(test)]
mod tests {
    use bc_core::ImportConfig;
    use bc_core::Importer as _;
    use bc_format_ofx::OfxImporter;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn v1_two_transactions() {
        let bytes = include_bytes!("fixtures/basic_v1.ofx");
        let txs = OfxImporter::new()
            .import(bytes, &ImportConfig::default())
            .expect("import v1");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].date, date(2025, 1, 15));
        assert_eq!(txs[0].reference.as_deref(), Some("20250115001"));
        assert_eq!(txs[0].payee.as_deref(), Some("WOOLWORTHS"));
        assert_eq!(txs[0].description, "Grocery purchase");
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn v2_two_transactions() {
        let bytes = include_bytes!("fixtures/basic_v2.ofx");
        let txs = OfxImporter::new()
            .import(bytes, &ImportConfig::default())
            .expect("import v2");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].date, date(2025, 1, 15));
        assert_eq!(txs[1].date, date(2025, 1, 16));
    }

    #[test]
    fn v1_and_v2_produce_same_transactions() {
        let v1 = include_bytes!("fixtures/basic_v1.ofx");
        let v2 = include_bytes!("fixtures/basic_v2.ofx");
        let txs_v1 = OfxImporter::new()
            .import(v1, &ImportConfig::default())
            .expect("v1");
        let txs_v2 = OfxImporter::new()
            .import(v2, &ImportConfig::default())
            .expect("v2");
        assert_eq!(txs_v1.len(), txs_v2.len());
        for (t1, t2) in txs_v1.iter().zip(txs_v2.iter()) {
            assert_eq!(t1.date, t2.date);
            assert_eq!(t1.amount, t2.amount);
            assert_eq!(t1.reference, t2.reference);
        }
    }
}
