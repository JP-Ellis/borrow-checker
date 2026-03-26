//! Shared domain types for BorrowChecker.
//!
//! This crate is the shared vocabulary for the whole workspace.
//! It has no internal dependencies and no I/O.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

/// Defines a typed ID newtype wrapping [`mti::MagicTypeId`] with a fixed prefix.
///
/// Used internally by each entity module to co-locate its ID type.
#[doc(hidden)]
#[macro_export]
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

mod account;
mod commodity;
mod event;
mod import_batch;
mod money;
mod period;
mod profile;
mod tag;
mod transaction;

pub use account::Account;
pub use account::AccountBuilder;
pub use account::AccountId;
pub use account::Kind as AccountKind;
pub use account::Type as AccountType;
pub use account::ValidationError as AccountValidationError;
pub use commodity::Commodity;
pub use commodity::CommodityBuilder;
pub use commodity::CommodityId;
pub use event::EventId;
pub use import_batch::ImportBatchId;
pub use money::Amount;
pub use money::CommodityCode;
pub use money::Decimal;
pub use period::Period;
pub use profile::ProfileId;
pub use tag::Forest as TagForest;
pub use tag::ParseError as TagPathError;
pub use tag::Path as TagPath;
pub use tag::Tag;
pub use tag::TagBuilder;
pub use tag::TagId;
pub use transaction::Cost;
pub use transaction::CostBuilder;
pub use transaction::Link as TransactionLink;
pub use transaction::LinkBuilder as TransactionLinkBuilder;
pub use transaction::LinkType as TransactionLinkType;
pub use transaction::Posting;
pub use transaction::PostingBuilder;
pub use transaction::PostingId;
pub use transaction::Status as TransactionStatus;
pub use transaction::Transaction;
pub use transaction::TransactionBuilder;
pub use transaction::TransactionId;
pub use transaction::TransactionLinkId;
