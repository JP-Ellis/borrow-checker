//! Budget calculation engine: actuals, rollover, and envelope status.

use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Envelope;
use bc_models::EnvelopeId;
use bc_models::TransactionStatus;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::to_db_str;
use crate::envelope::Service as EnvelopeService;

/// Computed budget status for one envelope in one period.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EnvelopeStatus {
    /// The envelope this status is for.
    pub envelope: Envelope,
    /// Period start date (inclusive).
    pub period_start: Date,
    /// Period end date (exclusive).
    pub period_end: Date,
    /// Total allocated for this period (zero if no allocation record exists).
    pub allocated: Decimal,
    /// Commodity of all monetary values in this status.
    pub commodity: CommodityCode,
    /// Sum of postings assigned to this envelope in the period.
    pub actuals: Decimal,
    /// Balance rolled over from the previous period (zero for `ResetToZero` policy).
    pub rollover: Decimal,
    /// Funds available: `allocated + rollover - actuals`.
    pub available: Decimal,
}

/// Computes budget actuals, rollover, and status for envelopes.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Engine {
    /// Creates a new [`Engine`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Computes the budget status for `envelope` as of `as_of`.
    ///
    /// The period is determined by `envelope.period().range_containing(as_of)`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for(&self, envelope: &Envelope, as_of: Date) -> BcResult<EnvelopeStatus> {
        let (period_start, period_end) = envelope.period().range_containing(as_of);

        let commodity = envelope
            .allocation_target()
            .map_or_else(|| CommodityCode::new("AUD"), |a| a.commodity().clone());

        let env_svc = EnvelopeService::new(self.pool.clone());
        let allocation = env_svc.get_allocation(envelope.id(), period_start).await?;
        let allocated = allocation
            .as_ref()
            .map_or(Decimal::ZERO, |a| a.amount().value());

        let actuals = self
            .sum_actuals(envelope.id(), period_start, period_end, &commodity)
            .await?;
        let rollover = self
            .rollover_for(envelope, period_start, &commodity)
            .await?;
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "budget arithmetic on Decimal values; overflow is handled via checked_add in sum_actuals"
        )]
        let available = allocated + rollover - actuals;

        Ok(EnvelopeStatus {
            envelope: envelope.clone(),
            period_start,
            period_end,
            allocated,
            commodity,
            actuals,
            rollover,
            available,
        })
    }

    /// Computes budget status for multiple envelopes as of `as_of`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_all(
        &self,
        envelopes: &[Envelope],
        as_of: Date,
    ) -> BcResult<Vec<EnvelopeStatus>> {
        let mut out = Vec::with_capacity(envelopes.len());
        for env in envelopes {
            out.push(self.status_for(env, as_of).await?);
        }
        Ok(out)
    }

    /// Sums the amounts of all non-voided postings assigned to `envelope_id`
    /// with transaction date in `[period_start, period_end)`.
    ///
    /// Only postings whose commodity matches `commodity` are included.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    async fn sum_actuals(
        &self,
        envelope_id: &EnvelopeId,
        period_start: Date,
        period_end: Date,
        commodity: &CommodityCode,
    ) -> BcResult<Decimal> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT p.amount
             FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.envelope_id = ?
               AND p.commodity   = ?
               AND t.date        >= ?
               AND t.date        <  ?
               AND t.status      != ?",
        )
        .bind(envelope_id.to_string())
        .bind(commodity.as_str())
        .bind(period_start.to_string())
        .bind(period_end.to_string())
        .bind(&voided_str)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().try_fold(Decimal::ZERO, |acc, (amt_str,)| {
            let d = amt_str.parse::<Decimal>().map_err(|e| {
                BcError::BadData(format!("invalid posting amount '{amt_str}': {e}"))
            })?;
            acc.checked_add(d)
                .ok_or_else(|| BcError::BadData("actuals sum overflow".into()))
        })
    }

    /// Computes the rollover balance from the period immediately before `period_start`.
    ///
    /// For `ResetToZero`: always returns [`Decimal::ZERO`].
    /// For `CarryForward`: returns `prev_allocated - prev_actuals` (can be negative).
    /// For `CapAtTarget`: returns `min(max(0, prev_allocated - prev_actuals), allocation_target)`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    async fn rollover_for(
        &self,
        envelope: &Envelope,
        period_start: Date,
        commodity: &CommodityCode,
    ) -> BcResult<Decimal> {
        if matches!(
            envelope.rollover_policy(),
            bc_models::RolloverPolicy::ResetToZero
        ) {
            return Ok(Decimal::ZERO);
        }

        let prev_period_date = period_start
            .checked_sub(jiff::Span::new().days(1_i32))
            .map_err(|e| BcError::BadData(format!("period underflow: {e}")))?;
        let (prev_start, prev_end) = envelope.period().range_containing(prev_period_date);

        let env_svc = EnvelopeService::new(self.pool.clone());
        let prev_alloc = env_svc.get_allocation(envelope.id(), prev_start).await?;
        let prev_allocated = prev_alloc.map_or(Decimal::ZERO, |a| a.amount().value());

        let prev_actuals = self
            .sum_actuals(envelope.id(), prev_start, prev_end, commodity)
            .await?;

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "rollover surplus: Decimal subtraction on budget values; magnitude is bounded by allocation amounts"
        )]
        let surplus = prev_allocated - prev_actuals;

        Ok(match envelope.rollover_policy() {
            bc_models::RolloverPolicy::CarryForward => surplus,
            bc_models::RolloverPolicy::CapAtTarget => {
                let cap = envelope
                    .allocation_target()
                    .map_or(Decimal::MAX, bc_models::Amount::value);
                surplus.max(Decimal::ZERO).min(cap)
            }
            // ResetToZero is already handled by the early return above; the wildcard
            // arm covers any future #[non_exhaustive] variants.
            bc_models::RolloverPolicy::ResetToZero | _ => Decimal::ZERO,
        })
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::envelope::CreateParams;
    use crate::envelope::Service as EnvelopeService;

    async fn make_envelope(
        svc: &EnvelopeService,
        name: &str,
        rollover: RolloverPolicy,
    ) -> bc_models::Envelope {
        svc.create(CreateParams {
            name: name.to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            allocation_target: Some(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            )),
            period: Period::Monthly,
            rollover_policy: rollover,
            account_ids: vec![],
        })
        .await
        .expect("create envelope")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_with_no_allocation_and_no_actuals(pool: sqlx::SqlitePool) {
        let env_svc = EnvelopeService::new(pool.clone());
        let engine = Engine::new(pool.clone());
        let env = make_envelope(&env_svc, "Groceries", RolloverPolicy::CarryForward).await;

        let status = engine
            .status_for(&env, Date::constant(2026, 3, 15))
            .await
            .expect("status");

        assert_eq!(status.allocated, Decimal::ZERO);
        assert_eq!(status.actuals, Decimal::ZERO);
        assert_eq!(status.rollover, Decimal::ZERO);
        assert_eq!(status.available, Decimal::ZERO);
        assert_eq!(status.period_start, Date::constant(2026, 3, 1));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_with_allocation_and_no_actuals(pool: sqlx::SqlitePool) {
        let env_svc = EnvelopeService::new(pool.clone());
        let engine = Engine::new(pool.clone());
        let env = make_envelope(&env_svc, "Groceries", RolloverPolicy::ResetToZero).await;

        env_svc
            .allocate(
                env.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate");

        let status = engine
            .status_for(&env, Date::constant(2026, 3, 15))
            .await
            .expect("status");

        assert_eq!(status.allocated, Decimal::from(500_i32));
        assert_eq!(status.actuals, Decimal::ZERO);
        assert_eq!(status.available, Decimal::from(500_i32));
    }
}
