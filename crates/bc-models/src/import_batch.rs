//! Import batch entity identifier.

use core::fmt;
use core::str::FromStr;

use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

crate::define_id!(ImportBatchId, "import_batch");

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    macro_rules! assert_json_snapshot {
        ($value:expr, @$snapshot:literal) => {
            insta::with_settings!({ filters => vec![("_[a-z0-9]{26}", "_[id]")] }, {
                insta::assert_json_snapshot!($value, @$snapshot);
            });
        }
    }

    #[test]
    fn import_batch_id_has_correct_prefix() {
        assert!(
            ImportBatchId::new()
                .to_string()
                .starts_with("import_batch_")
        );
    }

    #[test]
    fn import_batch_id_roundtrip_display_parse() {
        let id = ImportBatchId::new();
        let parsed: ImportBatchId = id.to_string().parse().expect("valid ImportBatchId");
        assert_eq!(id, parsed);
    }

    #[test]
    fn import_batch_id_serialize() {
        assert_json_snapshot!(ImportBatchId::new(), @r#""import_batch_[id]""#);
    }
}
