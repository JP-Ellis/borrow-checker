//! Envelope budgeting domain types.
//!
//! An *envelope* represents a budgeting category with an optional allocation
//! target and rollover policy. Envelopes can be organised into [`Group`]s for
//! hierarchical display. [`Allocation`] records the amount budgeted for a
//! specific envelope in a given period.
//!
//! # Re-exports
//!
//! The crate root re-exports these types as:
//! - [`crate::Envelope`], [`crate::EnvelopeId`], [`crate::EnvelopeBuilder`]
//! - [`crate::EnvelopeGroup`], [`crate::EnvelopeGroupId`], [`crate::EnvelopeGroupBuilder`]
//! - [`crate::Allocation`], [`crate::AllocationId`], [`crate::AllocationBuilder`]
//! - [`crate::RolloverPolicy`]

use jiff::Timestamp;
use jiff::civil::Date;

crate::define_id!(EnvelopeId, "envelope");
crate::define_id!(EnvelopeGroupId, "envelope_group");
crate::define_id!(AllocationId, "allocation");

/// Determines what happens to unspent (or overspent) funds at the end of a period.
///
/// Re-exported from the crate root as [`crate::RolloverPolicy`].
///
/// # Example
///
/// ```
/// use bc_models::RolloverPolicy;
///
/// let policy = RolloverPolicy::CarryForward;
/// assert_eq!(policy, RolloverPolicy::CarryForward);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum RolloverPolicy {
    /// Unspent funds roll into the next period's balance.
    CarryForward,
    /// The envelope resets to zero at the start of each period.
    ResetToZero,
    /// Unspent funds carry forward but are capped at the allocation target.
    CapAtTarget,
}

/// A named group that organises envelopes into a hierarchy.
///
/// Groups can themselves be nested via [`Group::parent_id`].
///
/// Re-exported from the crate root as [`crate::EnvelopeGroup`].
///
/// # Example
///
/// ```
/// use bc_models::{EnvelopeGroup, EnvelopeGroupId};
/// use jiff::Timestamp;
///
/// let group = EnvelopeGroup::builder()
///     .id(EnvelopeGroupId::new())
///     .name("Housing")
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(group.name(), "Housing");
/// assert!(!group.is_archived());
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Group {
    /// Stable, opaque identifier for this group (a prefixed `UUIDv7`).
    /// Only supply this when re-hydrating a record from storage.
    id: EnvelopeGroupId,

    /// Display name for this envelope group.
    #[builder(into)]
    name: String,

    /// Parent group ID, or `None` if this is a root group.
    parent_id: Option<EnvelopeGroupId>,

    /// Timestamp recorded when this group was first persisted.
    created_at: Timestamp,

    /// Timestamp at which this group was archived, or `None` if still active.
    archived_at: Option<Timestamp>,
}

impl Group {
    /// Returns the group ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &EnvelopeGroupId {
        &self.id
    }

    /// Returns the group name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parent group ID, if any.
    #[inline]
    #[must_use]
    pub fn parent_id(&self) -> Option<&EnvelopeGroupId> {
        self.parent_id.as_ref()
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns `true` if this group has been archived.
    #[inline]
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// Returns the archive timestamp, if archived.
    #[inline]
    #[must_use]
    pub fn archived_at(&self) -> Option<&Timestamp> {
        self.archived_at.as_ref()
    }
}

/// A budgeting envelope — a named category with an optional allocation target.
///
/// When [`Envelope::allocation_target`] is `None` the envelope operates in
/// *category tracking* mode: transactions are categorised against it but no
/// budget target is enforced. Use [`Envelope::is_tracking_only`] to test this.
///
/// Re-exported from the crate root as [`crate::Envelope`].
///
/// # Example
///
/// ```
/// use bc_models::{Envelope, EnvelopeId, RolloverPolicy, Period, CommodityCode};
/// use jiff::Timestamp;
///
/// let env = Envelope::builder()
///     .id(EnvelopeId::new())
///     .name("Groceries")
///     .commodity(CommodityCode::new("AUD"))
///     .period(Period::Monthly)
///     .rollover_policy(RolloverPolicy::CarryForward)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(env.name(), "Groceries");
/// assert!(env.is_tracking_only());
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Envelope {
    /// Stable, opaque identifier for this envelope (a prefixed `UUIDv7`).
    /// Only supply this when re-hydrating a record from storage.
    id: EnvelopeId,

    /// Display name for this envelope.
    #[builder(into)]
    name: String,

    /// Parent group ID, or `None` if this envelope is not in any group.
    ///
    /// Named `parent_id` (rather than `group_id`) to avoid
    /// `clippy::module_name_repetitions`.
    parent_id: Option<EnvelopeGroupId>,

    /// Optional icon identifier (e.g. an emoji or icon name).
    #[builder(into)]
    icon: Option<String>,

    /// Optional colour code (e.g. a CSS hex colour).
    #[builder(into)]
    colour: Option<String>,

    /// The commodity (currency) this envelope tracks.
    ///
    /// All actuals, allocations, and rollovers for this envelope are
    /// denominated in this commodity. When [`Envelope::allocation_target`]
    /// is set, its commodity **must** match this field.
    commodity: crate::money::CommodityCode,

    /// Optional allocation target amount.
    ///
    /// `None` places the envelope in category-tracking mode; the envelope
    /// records spending but enforces no budget. `Some(amount)` sets the
    /// target allocation per [`Envelope::period`].
    allocation_target: Option<crate::money::Amount>,

    /// The recurring period over which the allocation target is measured.
    period: crate::period::Period,

    /// What happens to unspent funds at the end of each period.
    rollover_policy: RolloverPolicy,

    /// Account IDs whose transactions are tracked against this envelope.
    #[builder(default)]
    account_ids: Vec<crate::AccountId>,

    /// Timestamp recorded when this envelope was first persisted.
    created_at: Timestamp,

    /// Timestamp at which this envelope was archived, or `None` if still active.
    archived_at: Option<Timestamp>,
}

impl Envelope {
    /// Returns the envelope ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &EnvelopeId {
        &self.id
    }

    /// Returns the envelope name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parent group ID, if any.
    #[inline]
    #[must_use]
    pub fn parent_id(&self) -> Option<&EnvelopeGroupId> {
        self.parent_id.as_ref()
    }

    /// Returns the icon identifier, if any.
    #[inline]
    #[must_use]
    pub fn icon(&self) -> Option<&str> {
        self.icon.as_deref()
    }

    /// Returns the colour code, if any.
    #[inline]
    #[must_use]
    pub fn colour(&self) -> Option<&str> {
        self.colour.as_deref()
    }

    /// Returns the commodity this envelope is denominated in.
    #[inline]
    #[must_use]
    pub fn commodity(&self) -> &crate::money::CommodityCode {
        &self.commodity
    }

    /// Returns the allocation target, if set.
    #[inline]
    #[must_use]
    pub fn allocation_target(&self) -> Option<&crate::money::Amount> {
        self.allocation_target.as_ref()
    }

    /// Returns `true` when no allocation target is set (category tracking mode).
    #[inline]
    #[must_use]
    pub fn is_tracking_only(&self) -> bool {
        self.allocation_target.is_none()
    }

    /// Returns the budget period for this envelope.
    #[inline]
    #[must_use]
    pub fn period(&self) -> &crate::period::Period {
        &self.period
    }

    /// Returns the rollover policy for this envelope.
    #[inline]
    #[must_use]
    pub fn rollover_policy(&self) -> RolloverPolicy {
        self.rollover_policy
    }

    /// Returns the account IDs tracked by this envelope.
    #[inline]
    #[must_use]
    pub fn account_ids(&self) -> &[crate::AccountId] {
        &self.account_ids
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns `true` if this envelope has been archived.
    #[inline]
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// Returns the archive timestamp, if archived.
    #[inline]
    #[must_use]
    pub fn archived_at(&self) -> Option<&Timestamp> {
        self.archived_at.as_ref()
    }
}

/// A record of the amount budgeted for an envelope in a specific period.
///
/// Re-exported from the crate root as [`crate::Allocation`].
///
/// # Example
///
/// ```
/// use bc_models::{Allocation, AllocationId, EnvelopeId, Amount, CommodityCode, Decimal};
/// use jiff::{Timestamp, civil::Date};
///
/// let alloc = Allocation::builder()
///     .id(AllocationId::new())
///     .envelope_id(EnvelopeId::new())
///     .period_start(Date::constant(2024, 1, 1))
///     .amount(Amount::new(Decimal::from(500), CommodityCode::new("AUD")))
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(alloc.period_start(), Date::constant(2024, 1, 1));
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Allocation {
    /// Stable, opaque identifier for this allocation (a prefixed `UUIDv7`).
    /// Only supply this when re-hydrating a record from storage.
    id: AllocationId,

    /// The envelope this allocation applies to.
    envelope_id: EnvelopeId,

    /// The calendar date on which this budget period begins.
    period_start: Date,

    /// The amount budgeted for this period.
    amount: crate::money::Amount,

    /// Timestamp recorded when this allocation was first persisted.
    created_at: Timestamp,
}

impl Allocation {
    /// Returns the allocation ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &AllocationId {
        &self.id
    }

    /// Returns the envelope ID this allocation belongs to.
    #[inline]
    #[must_use]
    pub fn envelope_id(&self) -> &EnvelopeId {
        &self.envelope_id
    }

    /// Returns the start date of the budget period this allocation covers.
    #[inline]
    #[must_use]
    pub fn period_start(&self) -> Date {
        self.period_start
    }

    /// Returns the budgeted amount.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> &crate::money::Amount {
        &self.amount
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn envelope_id_round_trips_display_from_str() {
        let id = EnvelopeId::new();
        let s = id.to_string();
        let parsed: EnvelopeId = s.parse().expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn envelope_group_id_round_trips_display_from_str() {
        let id = EnvelopeGroupId::new();
        let s = id.to_string();
        let parsed: EnvelopeGroupId = s.parse().expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn allocation_id_round_trips_display_from_str() {
        let id = AllocationId::new();
        let s = id.to_string();
        let parsed: AllocationId = s.parse().expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rollover_policy_serialises_as_snake_case() {
        let carry = serde_json::to_string(&RolloverPolicy::CarryForward).expect("ser");
        let reset = serde_json::to_string(&RolloverPolicy::ResetToZero).expect("ser");
        let cap = serde_json::to_string(&RolloverPolicy::CapAtTarget).expect("ser");
        assert_eq!(carry, r#""carry_forward""#);
        assert_eq!(reset, r#""reset_to_zero""#);
        assert_eq!(cap, r#""cap_at_target""#);
    }

    #[test]
    fn envelope_without_target_is_tracking_only() {
        use jiff::Timestamp;

        use crate::CommodityCode;
        use crate::Period;
        let env = Envelope::builder()
            .id(EnvelopeId::new())
            .name("Dining Out")
            .commodity(CommodityCode::new("AUD"))
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        assert!(env.is_tracking_only());
    }

    #[test]
    fn envelope_with_target_is_not_tracking_only() {
        use jiff::Timestamp;

        use crate::Amount;
        use crate::CommodityCode;
        use crate::Decimal;
        use crate::Period;
        let env = Envelope::builder()
            .id(EnvelopeId::new())
            .name("Groceries")
            .commodity(CommodityCode::new("AUD"))
            .allocation_target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .created_at(Timestamp::now())
            .build();
        assert!(!env.is_tracking_only());
    }
}
