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
#[derive(Debug, Clone, serde::Serialize)]
#[non_exhaustive]
pub struct EnvelopeStatus {
    /// The envelope this status is for.
    pub envelope: Envelope,
    /// Period start date (inclusive).
    pub period_start: Date,
    /// Period end date (exclusive).
    pub period_end: Date,
    /// The viewing window for which this status was computed.
    pub window: bc_models::BudgetWindow,
    /// Allocated amount, pro-rated to the window duration.
    pub allocated: Decimal,
    /// Commodity of all monetary values in this status, if the envelope has one set.
    ///
    /// When `None` the envelope tracks across multiple commodities and the monetary
    /// figures (`allocated`, `actuals`, `available`) are summed without commodity
    /// filtering.
    pub commodity: Option<CommodityCode>,
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

    /// Computes the budget status for `envelope` over an explicit [`bc_models::BudgetWindow`].
    ///
    /// The allocation is pro-rated: `prorated = allocation × (window_days / natural_period_days)`.
    /// Actuals are summed only within `[window.start, window.end)`.
    /// Rollover is computed from the natural period boundary (unchanged).
    ///
    /// # Arguments
    ///
    /// * `envelope` - The envelope to compute status for.
    /// * `window`   - The date range to query.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for_window(
        &self,
        envelope: &Envelope,
        window: bc_models::BudgetWindow,
    ) -> BcResult<EnvelopeStatus> {
        let (period_start, period_end) = envelope.period().range_containing(window.start);
        let commodity: Option<CommodityCode> = envelope.commodity().cloned();

        let env_svc = EnvelopeService::new(self.pool.clone());
        let allocation = env_svc.get_allocation(envelope.id(), period_start).await?;
        let full_allocated = allocation
            .as_ref()
            .map_or(Decimal::ZERO, |a| a.amount().value());

        // Pro-rate the allocation to the window duration.
        let window_days = window.days();
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date returns a Span; get_days() is safe for any realistic period"
        )]
        let period_days = i64::from((period_end - period_start).get_days());

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "pro-rating multiplication on Decimal; precision loss is acceptable for budget display"
        )]
        let allocated = if period_days == 0 {
            full_allocated
        } else {
            let ratio = Decimal::from(window_days) / Decimal::from(period_days);
            (full_allocated * ratio).round_dp(2)
        };

        let actuals = self
            .sum_actuals(envelope.id(), window.start, window.end, commodity.as_ref())
            .await?;
        let rollover = self
            .rollover_for(envelope, period_start, commodity.as_ref())
            .await?;

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "budget arithmetic on Decimal values; overflow handled via checked_add in sum_actuals"
        )]
        let available = allocated + rollover - actuals;

        Ok(EnvelopeStatus {
            envelope: envelope.clone(),
            period_start,
            period_end,
            window,
            allocated,
            commodity,
            actuals,
            rollover,
            available,
        })
    }

    /// Computes the budget status for `envelope` as of `as_of`.
    ///
    /// The period is determined by `envelope.period().range_containing(as_of)`.
    /// Delegates to [`Engine::status_for_window`] with the full natural period as the window.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn status_for(&self, envelope: &Envelope, as_of: Date) -> BcResult<EnvelopeStatus> {
        let (start, end) = envelope.period().range_containing(as_of);
        let label = format!("{start} \u{2013} {end}");
        self.status_for_window(envelope, bc_models::BudgetWindow::custom(start, end, label))
            .await
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
    /// When `commodity` is `Some`, only postings whose commodity matches are included.
    /// When `commodity` is `None`, all postings for the envelope are summed regardless
    /// of commodity (multi-commodity tracking envelopes).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    async fn sum_actuals(
        &self,
        envelope_id: &EnvelopeId,
        period_start: Date,
        period_end: Date,
        commodity: Option<&CommodityCode>,
    ) -> BcResult<Decimal> {
        let voided_str = to_db_str(TransactionStatus::Voided)?;
        let rows: Vec<(String,)> = if let Some(comm) = commodity {
            sqlx::query_as(
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
            .bind(comm.as_str())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(&voided_str)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as(
                "SELECT p.amount
                 FROM postings p
                 JOIN transactions t ON t.id = p.transaction_id
                 WHERE p.envelope_id = ?
                   AND t.date        >= ?
                   AND t.date        <  ?
                   AND t.status      != ?",
            )
            .bind(envelope_id.to_string())
            .bind(period_start.to_string())
            .bind(period_end.to_string())
            .bind(&voided_str)
            .fetch_all(&self.pool)
            .await?
        };

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
    /// For `CarryForward`: recursively returns `prev_allocated + prev_rollover - prev_actuals`
    /// (can be negative).
    /// For `CapAtTarget`: returns
    /// `min(max(0, prev_allocated + prev_rollover - prev_actuals), allocation_target)`.
    ///
    /// When `commodity` is `None`, actuals are summed across all commodities.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    fn rollover_for<'a>(
        &'a self,
        envelope: &'a Envelope,
        period_start: Date,
        commodity: Option<&'a CommodityCode>,
    ) -> core::pin::Pin<Box<dyn core::future::Future<Output = BcResult<Decimal>> + Send + 'a>> {
        Box::pin(async move {
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
            let prev_allocated = prev_alloc
                .as_ref()
                .map_or(Decimal::ZERO, |a| a.amount().value());

            let prev_actuals = self
                .sum_actuals(envelope.id(), prev_start, prev_end, commodity)
                .await?;

            // Base case: no allocation record and no spending in this period means there
            // is nothing to carry forward from here or any earlier period.
            if prev_alloc.is_none() && prev_actuals == Decimal::ZERO {
                return Ok(Decimal::ZERO);
            }

            let prev_rollover = self.rollover_for(envelope, prev_start, commodity).await?;

            #[expect(
                clippy::arithmetic_side_effects,
                reason = "budget arithmetic on Decimal values bounded by allocation amounts"
            )]
            let surplus = prev_allocated + prev_rollover - prev_actuals;

            Ok(match envelope.rollover_policy() {
                bc_models::RolloverPolicy::CarryForward => surplus,
                bc_models::RolloverPolicy::CapAtTarget => {
                    #[expect(
                        clippy::expect_used,
                        reason = "CapAtTarget envelopes are validated to have allocation_target \
                                  at creation time; Service::create() enforces this invariant"
                    )]
                    let cap = envelope
                        .allocation_target()
                        .expect(
                            "CapAtTarget envelope must have allocation_target; \
                             Service::create() validates this invariant",
                        )
                        .value();
                    surplus.max(Decimal::ZERO).min(cap)
                }
                // ResetToZero is already handled by the early return above; the wildcard
                // arm covers any future #[non_exhaustive] variants added to bc-models.
                bc_models::RolloverPolicy::ResetToZero => Decimal::ZERO,
                _ => {
                    tracing::warn!(
                        policy = ?envelope.rollover_policy(),
                        "unrecognised rollover policy variant — defaulting to zero; \
                         add a match arm if a new RolloverPolicy variant was introduced"
                    );
                    Decimal::ZERO
                }
            })
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
    use crate::envelope::Service as EnvelopeService;

    async fn make_envelope(
        svc: &EnvelopeService,
        name: &str,
        rollover: RolloverPolicy,
    ) -> bc_models::Envelope {
        svc.create()
            .name(name.to_owned())
            .commodity(CommodityCode::new("AUD"))
            .allocation_target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover_policy(rollover)
            .call()
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

    /// A tracking-only envelope in USD must report USD actuals, not zero (AUD default regression).
    #[sqlx::test(migrations = "./migrations")]
    async fn tracking_envelope_reports_actuals_in_its_own_commodity(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Posting;
        use bc_models::PostingId;
        use bc_models::Transaction;
        use bc_models::TransactionId;
        use bc_models::TransactionStatus;

        use crate::account::Service as AccountService;
        use crate::transaction::Service as TxService;

        let acct_svc = AccountService::new(pool.clone());
        let checking = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        let expense = acct_svc
            .create()
            .name("Dining")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create expense");

        let env_svc = EnvelopeService::new(pool.clone());
        let env = env_svc
            .create()
            .name("Dining".to_owned())
            .commodity(CommodityCode::new("USD"))
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create tracking envelope");

        let tx_svc = TxService::new(pool.clone());
        tx_svc
            .create(
                Transaction::builder()
                    .id(TransactionId::new())
                    .date(Date::constant(2026, 3, 10))
                    .description("Restaurant")
                    .status(TransactionStatus::Cleared)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(expense.clone())
                            .amount(Amount::new(
                                Decimal::from(42_i32),
                                CommodityCode::new("USD"),
                            ))
                            .envelope_id(env.id().clone())
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(
                                Decimal::from(-42_i32),
                                CommodityCode::new("USD"),
                            ))
                            .build(),
                    ])
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("create transaction");

        let engine = Engine::new(pool.clone());
        let status = engine
            .status_for(&env, Date::constant(2026, 3, 15))
            .await
            .expect("status");

        assert_eq!(status.commodity, Some(CommodityCode::new("USD")));
        assert_eq!(status.actuals, Decimal::from(42_i32));
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

    /// Carry-forward rollover must accumulate across multiple periods.
    ///
    /// Jan: allocated=500, actuals=300 → surplus=200 carried to Feb
    /// Feb: allocated=500, actuals=400, rollover=200 → available=300 carried to Mar
    /// Mar: rollover must be 300, not 100 (the naive `prev_allocated` - `prev_actuals`).
    #[sqlx::test(migrations = "./migrations")]
    async fn carry_forward_accumulates_across_three_periods(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Posting;
        use bc_models::PostingId;
        use bc_models::Transaction;
        use bc_models::TransactionId;
        use bc_models::TransactionStatus;

        use crate::account::Service as AccountService;
        use crate::transaction::Service as TxService;

        let acct_svc = AccountService::new(pool.clone());
        let checking = acct_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create checking account");
        let expense = acct_svc
            .create()
            .name("Groceries")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create expense account");

        let env_svc = EnvelopeService::new(pool.clone());
        let env = make_envelope(&env_svc, "Groceries", RolloverPolicy::CarryForward).await;

        // Allocate $500 for January
        env_svc
            .allocate(
                env.id(),
                Date::constant(2026, 1, 1),
                Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate Jan");

        // Allocate $500 for February
        env_svc
            .allocate(
                env.id(),
                Date::constant(2026, 2, 1),
                Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate Feb");

        let tx_svc = TxService::new(pool.clone());

        // January: spend $300 (surplus = 500 - 300 = 200 carried to Feb)
        tx_svc
            .create(
                Transaction::builder()
                    .id(TransactionId::new())
                    .date(Date::constant(2026, 1, 15))
                    .description("Jan groceries")
                    .status(TransactionStatus::Cleared)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(expense.clone())
                            .amount(Amount::new(
                                Decimal::from(300_i32),
                                CommodityCode::new("AUD"),
                            ))
                            .envelope_id(env.id().clone())
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(
                                Decimal::from(-300_i32),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("create Jan transaction");

        // February: spend $400 (available = 500 + 200 - 400 = 300 carried to Mar)
        tx_svc
            .create(
                Transaction::builder()
                    .id(TransactionId::new())
                    .date(Date::constant(2026, 2, 15))
                    .description("Feb groceries")
                    .status(TransactionStatus::Cleared)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(expense.clone())
                            .amount(Amount::new(
                                Decimal::from(400_i32),
                                CommodityCode::new("AUD"),
                            ))
                            .envelope_id(env.id().clone())
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(
                                Decimal::from(-400_i32),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("create Feb transaction");

        let engine = Engine::new(pool.clone());
        let status = engine
            .status_for(&env, Date::constant(2026, 3, 15))
            .await
            .expect("Mar status");

        // Rollover into March must be 300, not the naive 100 (500 - 400)
        assert_eq!(status.rollover, Decimal::from(300_i32));
        // Mar has no allocation and no actuals, so available == rollover
        assert_eq!(status.available, Decimal::from(300_i32));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_for_window_matches_full_natural_period(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use jiff::civil::date;

        let env_svc = EnvelopeService::new(pool.clone());
        let engine = Engine::new(pool.clone());
        let env = make_envelope(&env_svc, "Groceries", RolloverPolicy::ResetToZero).await;

        env_svc
            .allocate(
                env.id(),
                date(2026, 3, 1),
                Amount::new(Decimal::from(600_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate");

        // Window equals the full natural period — should match status_for
        let full_window =
            bc_models::BudgetWindow::custom(date(2026, 3, 1), date(2026, 4, 1), "March 2026");
        let ws = engine
            .status_for_window(&env, full_window)
            .await
            .expect("window status");
        let ts = engine
            .status_for(&env, date(2026, 3, 15))
            .await
            .expect("regular status");

        assert_eq!(ws.allocated, ts.allocated);
        assert_eq!(ws.actuals, ts.actuals);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn status_for_window_prorates_half_month(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use jiff::civil::date;

        let env_svc = EnvelopeService::new(pool.clone());
        let engine = Engine::new(pool.clone());
        let env = make_envelope(&env_svc, "Groceries", RolloverPolicy::ResetToZero).await;

        // Allocate $600 for April 2026 (30-day month)
        env_svc
            .allocate(
                env.id(),
                date(2026, 4, 1),
                Amount::new(Decimal::from(600_i32), CommodityCode::new("AUD")),
            )
            .await
            .expect("allocate");

        // Query only the first 15 days of April → expect ~$300 (15/30 * 600)
        let half_window =
            bc_models::BudgetWindow::custom(date(2026, 4, 1), date(2026, 4, 16), "Apr 1–15");
        let ws = engine
            .status_for_window(&env, half_window)
            .await
            .expect("window status");

        // 15 days out of 30 → 300
        assert_eq!(ws.allocated, Decimal::from(300_i32));
    }
}
