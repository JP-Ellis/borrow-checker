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
pub mod money;
mod period;
mod profile;
mod tag;
mod transaction;

// IDs still come from ids.rs while migration is in progress
pub mod ids;

pub mod settings;

pub use account::{Account, AccountType};
pub use commodity::{Commodity, CommodityId};
pub use event::EventId;
pub use ids::{AccountId, PostingId, TransactionId};
pub use import_batch::ImportBatchId;
pub use money::{Amount, CommodityCode, Decimal};
pub use period::Period;
pub use profile::ProfileId;
pub use settings::GlobalSettings;
pub use tag::{ParseError as TagPathError, Path as TagPath};
pub use transaction::{Posting, Transaction, TransactionStatus};
