//! Import batch entity identifier.

crate::define_id!(ImportBatchId, "import_batch");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_batch_id_has_correct_prefix() {
        assert!(
            ImportBatchId::new()
                .to_string()
                .starts_with("import_batch_")
        );
    }
}
