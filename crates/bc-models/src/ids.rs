//! Typed ID newtypes for all BorrowChecker domain entities.
//!
//! Each ID type wraps a [`MagicTypeId`] from the `mti` crate,
//! producing human-readable prefixed IDs like `account_01j...`.

use core::{fmt, str::FromStr};

use mti::prelude::*;
use serde::{Deserialize, Serialize};

/// Defines a typed ID newtype wrapping [`MagicTypeId`] with a fixed prefix.
macro_rules! define_id {
    ($name:ident, $prefix:literal) => {
        #[doc = concat!("A unique identifier for a `", stringify!($name), "` entity.")]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(MagicTypeId);

        impl $name {
            #[doc = concat!("Creates a new unique `", stringify!($name), "`.")]
            #[inline]
            #[must_use]
            pub fn new() -> Self {
                Self($prefix.create_type_id::<V7>())
            }
        }

        impl Default for $name {
            #[inline]
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            #[inline]
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = String;

            #[inline]
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let id = MagicTypeId::from_str(s)
                    .map_err(|e| format!("invalid {}: {e}", stringify!($name)))?;
                let prefix = id
                    .prefix_str()
                    .map_err(|e| format!("invalid {} prefix: {e}", stringify!($name)))?;
                if prefix != $prefix {
                    return Err(format!("expected prefix '{}', got '{}'", $prefix, prefix));
                }
                Ok(Self(id))
            }
        }
    };
}

define_id!(AccountId, "account");
define_id!(EventId, "event");
define_id!(ImportBatchId, "import_batch");
define_id!(PostingId, "posting");
define_id!(ProfileId, "profile");
define_id!(TransactionId, "transaction");

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

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
    fn event_id_has_correct_prefix() {
        let id = EventId::new();
        assert!(id.to_string().starts_with("event_"));
    }

    #[test]
    fn event_id_roundtrip_display_parse() {
        let id = EventId::new();
        let s = id.to_string();
        let parsed: EventId = s.parse().expect("valid EventId string");
        assert_eq!(id, parsed);
    }

    #[test]
    fn import_batch_id_has_correct_prefix() {
        let id = ImportBatchId::new();
        assert!(id.to_string().starts_with("import_batch_"));
    }

    #[test]
    fn import_batch_id_roundtrip_display_parse() {
        let id = ImportBatchId::new();
        let s = id.to_string();
        let parsed: ImportBatchId = s.parse().expect("valid ImportBatchId string");
        assert_eq!(id, parsed);
    }

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
    fn profile_id_has_correct_prefix() {
        let id = ProfileId::new();
        assert!(id.to_string().starts_with("profile_"));
    }

    #[test]
    fn profile_id_roundtrip_display_parse() {
        let id = ProfileId::new();
        let s = id.to_string();
        let parsed: ProfileId = s.parse().expect("valid ProfileId string");
        assert_eq!(id, parsed);
    }

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
}
