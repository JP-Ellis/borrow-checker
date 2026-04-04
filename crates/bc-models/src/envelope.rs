//! Envelope budgeting domain types.
//!
//! An *envelope* represents a budgeting category with an optional allocation
//! target and rollover policy. Envelopes form an arbitrary-depth tree via
//! [`Envelope::parent_id`] — a parent envelope aggregates its children's
//! actuals and allocations. [`Allocation`] records the amount budgeted for a
//! specific envelope in a given period.
//!
//! # Re-exports
//!
//! The crate root re-exports these types as:
//! - [`crate::Envelope`], [`crate::EnvelopeId`], [`crate::EnvelopeBuilder`]
//! - [`crate::Allocation`], [`crate::AllocationId`], [`crate::AllocationBuilder`]
//! - [`crate::RolloverPolicy`]

use jiff::Timestamp;
use jiff::civil::Date;

crate::define_id!(EnvelopeId, "envelope");
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

/// A budgeting envelope — a named category with an optional allocation target.
///
/// Envelopes form an arbitrary-depth tree via [`Envelope::parent_id`]. A
/// parent envelope aggregates its children's actuals and allocations for
/// roll-up reporting. Leaf envelopes (no children) are the units against which
/// postings are assigned.
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
/// use bc_models::{Envelope, RolloverPolicy, Period};
/// use jiff::Timestamp;
///
/// let env = Envelope::builder()
///     .name("Groceries")
///     .period(Period::Monthly)
///     .rollover_policy(RolloverPolicy::CarryForward)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(env.name(), "Groceries");
/// assert!(env.is_tracking_only());
/// assert!(env.commodity().is_none());
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Envelope {
    /// Stable, opaque identifier for this envelope (a prefixed `UUIDv7`).
    ///
    /// Auto-generated when not supplied; only set this when re-hydrating a
    /// record from storage.
    #[builder(default)]
    id: EnvelopeId,

    /// Display name for this envelope.
    #[builder(into)]
    name: String,

    /// Parent envelope ID, or `None` if this is a root envelope.
    ///
    /// Envelopes form an arbitrary-depth tree. A parent envelope aggregates
    /// its children's actuals and allocations for roll-up budget views.
    parent_id: Option<EnvelopeId>,

    /// Optional icon identifier (e.g. an emoji or icon name).
    #[builder(into)]
    icon: Option<String>,

    /// Optional colour code (e.g. a CSS hex colour).
    #[builder(into)]
    colour: Option<String>,

    /// The commodity (currency) this envelope tracks, if set.
    ///
    /// When `Some`, actuals filtering and allocation validation are restricted
    /// to this commodity. When `None`, the envelope tracks across commodities
    /// and conversion is left to reporting time.
    ///
    /// When [`Envelope::allocation_target`] is set, its commodity **must**
    /// match this field (enforced at service level).
    commodity: Option<crate::money::CommodityCode>,

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

    /// Tags applied to this envelope for cross-cutting budget views.
    ///
    /// These are raw [`crate::TagId`] references. Use the tag service to
    /// resolve them to human-readable paths.
    #[builder(default)]
    tag_ids: Vec<crate::TagId>,

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

    /// Returns the parent envelope ID, if any.
    #[inline]
    #[must_use]
    pub fn parent_id(&self) -> Option<&EnvelopeId> {
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

    /// Returns the commodity this envelope is denominated in, if set.
    ///
    /// `None` means the envelope tracks across commodities; conversion is
    /// deferred to reporting time.
    #[inline]
    #[must_use]
    pub fn commodity(&self) -> Option<&crate::money::CommodityCode> {
        self.commodity.as_ref()
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

    /// Returns the tag IDs applied to this envelope.
    #[inline]
    #[must_use]
    pub fn tag_ids(&self) -> &[crate::TagId] {
        &self.tag_ids
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
/// use bc_models::{Allocation, EnvelopeId, Amount, CommodityCode, Decimal};
/// use jiff::{Timestamp, civil::Date};
///
/// let alloc = Allocation::builder()
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
    ///
    /// Auto-generated when not supplied; only set this when re-hydrating a
    /// record from storage.
    #[builder(default)]
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
    fn envelope_id_is_auto_generated_when_not_set() {
        use jiff::Timestamp;

        use crate::Period;
        let env = Envelope::builder()
            .name("Groceries")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        // id should be auto-generated — just verify it round-trips
        let s = env.id().to_string();
        assert!(
            s.starts_with("envelope_"),
            "id should have envelope_ prefix, got {s}"
        );
    }

    #[test]
    fn envelope_with_no_commodity_is_valid() {
        use jiff::Timestamp;

        use crate::Period;
        let env = Envelope::builder()
            .name("Cash")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        assert!(env.commodity().is_none());
    }

    #[test]
    fn envelope_has_parent_envelope_not_group() {
        use jiff::Timestamp;

        use crate::Period;
        let parent_id = EnvelopeId::new();
        let env = Envelope::builder()
            .name("Gym")
            .parent_id(parent_id.clone())
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .created_at(Timestamp::now())
            .build();
        assert_eq!(env.parent_id(), Some(&parent_id));
    }

    #[test]
    fn envelope_without_target_is_tracking_only() {
        use jiff::Timestamp;

        use crate::Period;
        let env = Envelope::builder()
            .name("Dining Out")
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

    #[test]
    fn allocation_id_is_auto_generated_when_not_set() {
        use jiff::Timestamp;
        use jiff::civil::Date;

        use crate::Amount;
        use crate::CommodityCode;
        use crate::Decimal;
        let alloc = Allocation::builder()
            .envelope_id(EnvelopeId::new())
            .period_start(Date::constant(2024, 1, 1))
            .amount(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .created_at(Timestamp::now())
            .build();
        let s = alloc.id().to_string();
        assert!(
            s.starts_with("allocation_"),
            "id should have allocation_ prefix, got {s}"
        );
    }
}
