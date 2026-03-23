//! Typed ID newtypes for all BorrowChecker domain entities.
//!
//! Each ID type wraps a [`MagicTypeId`] from the `mti` crate,
//! producing human-readable prefixed IDs like `account_01j...`.

use core::{fmt, str::FromStr};

use mti::prelude::*;
use serde::{Deserialize, Serialize};

crate::define_id!(AccountId, "account");
crate::define_id!(PostingId, "posting");
crate::define_id!(TransactionId, "transaction");

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Define a helper macro to redact the random suffix of the ID for
    /// deterministic snapshots.
    macro_rules! assert_json_snapshot {
        ($value:expr, @$snapshot:literal) => {
            insta::with_settings!({
                filters => vec![
                    ("_[a-z0-9]{26}", "_[id]"),
                ],
            }, {
                insta::assert_json_snapshot!($value, @$snapshot);
            });
        }
    }

    // MARK: AccountId

    #[test]
    fn account_id_has_correct_prefix() {
        let id = AccountId::new();
        assert!(id.to_string().starts_with("account_"));
    }

    #[test]
    fn account_id_roundtrip_display_parse() {
        let id = AccountId::new();
        let s = id.to_string();
        let parsed: AccountId = s.parse().expect("valid AccountId string");
        assert_eq!(id, parsed);
    }

    #[test]
    fn account_id_serialize() {
        let id = AccountId::new();
        assert_json_snapshot!(id, @r#""account_[id]""#);
    }

    // MARK: PostingId

    #[test]
    fn posting_id_has_correct_prefix() {
        let id = PostingId::new();
        assert!(id.to_string().starts_with("posting_"));
    }

    #[test]
    fn posting_id_roundtrip_display_parse() {
        let id = PostingId::new();
        let s = id.to_string();
        let parsed: PostingId = s.parse().expect("valid PostingId string");
        assert_eq!(id, parsed);
    }

    #[test]
    fn posting_id_serialize() {
        let id = PostingId::new();
        assert_json_snapshot!(id, @r#""posting_[id]""#);
    }

    // MARK: TransactionId

    #[test]
    fn transaction_id_has_correct_prefix() {
        let id = TransactionId::new();
        assert!(id.to_string().starts_with("transaction_"));
    }

    #[test]
    fn transaction_id_roundtrip_display_parse() {
        let id = TransactionId::new();
        let s = id.to_string();
        let parsed: TransactionId = s.parse().expect("valid TransactionId string");
        assert_eq!(id, parsed);
    }

    #[test]
    fn transaction_id_serialize() {
        let id = TransactionId::new();
        assert_json_snapshot!(id, @r#""transaction_[id]""#);
    }
}
