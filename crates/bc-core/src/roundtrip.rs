//! Round-trip export and apply: short transaction handles and the sidecar
//! file that maps them back to full transactions.

pub mod handle;
pub mod sidecar;

pub use handle::abbreviate;
pub use sidecar::Sidecar;
pub use sidecar::SidecarEntry;
pub use sidecar::database_id;
