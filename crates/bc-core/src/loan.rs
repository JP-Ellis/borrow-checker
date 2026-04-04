//! Loan terms persistence and amortization schedule calculation.

use bc_models::AccountId;
use bc_models::AmortizationRow;
use bc_models::CompoundingFrequency;
use bc_models::LoanId;
use bc_models::LoanTerms;
use bc_models::Period;
use jiff::civil::Date;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::events::Event;
use crate::events::insert_event;

/// Raw row returned by the `loan_terms` projection table.
#[derive(sqlx::FromRow)]
struct LoanTermsRow {
    /// UUID of the loan-terms record.
    id: String,
    /// Serialised [`rust_decimal::Decimal`] principal amount.
    principal: String,
    /// Serialised [`rust_decimal::Decimal`] annual interest rate.
    interest_rate: String,
    /// ISO 8601 date string for the loan start date.
    start_date: String,
    /// Stored as signed 64-bit integer (SQLite's native integer type).
    /// Validated to be in `u32` range on read. A negative value in the DB
    /// indicates data corruption.
    term_months: i64,
    /// Serialised [`Period`] as JSON string.
    repayment_frequency: String,
    /// ISO 4217 commodity code (e.g. `"AUD"`).
    commodity: String,
    /// ISO 8601 timestamp string for when the record was created.
    created_at: String,
    /// Compounding frequency string (e.g. `"daily"` or `"monthly"`).
    compounding_frequency: String,
}

/// Serialises a [`Period`] to a JSON string for storage.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if serialization fails.
fn period_to_db(period: &Period) -> BcResult<String> {
    serde_json::to_string(period).map_err(BcError::Serialisation)
}

/// Deserialises a [`Period`] from a JSON string.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if deserialization fails.
fn period_from_db(s: &str) -> BcResult<Period> {
    serde_json::from_str(s).map_err(BcError::Serialisation)
}

/// Serialises a [`CompoundingFrequency`] to a lowercase DB string.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if serialization fails.
fn compounding_to_db(cf: CompoundingFrequency) -> BcResult<String> {
    serde_json::to_string(&cf)
        .map_err(BcError::Serialisation)
        .map(|s| s.trim_matches('"').to_owned())
}

/// Deserialises a [`CompoundingFrequency`] from a DB string.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if deserialization fails.
fn compounding_from_db(s: &str) -> BcResult<CompoundingFrequency> {
    let json = format!("\"{s}\"");
    serde_json::from_str(&json).map_err(BcError::Serialisation)
}

/// Service for persisting loan terms and computing amortization schedules.
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    ///
    /// # Arguments
    ///
    /// * `pool` - The SQLite connection pool to use for persistence.
    ///
    /// # Returns
    ///
    /// A new [`Service`] instance.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Persists loan terms for an account (most recent record wins on read).
    ///
    /// Appends a [`Event::LoanTermsSet`] event atomically alongside the
    /// projection row. Offset account links are also inserted atomically.
    ///
    /// # Arguments
    ///
    /// * `terms` - The loan terms to persist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if `terms.principal()` is zero or negative.
    /// Returns [`BcError::BadData`] if `terms.annual_rate()` is negative.
    /// Returns [`BcError::BadData`] if `terms.term_months()` is zero.
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::InvalidAccountKind`] if the account is not a `Receivable`.
    /// Returns [`BcError`] on serialisation or database failure.
    #[inline]
    pub async fn set_loan_terms(&self, terms: &LoanTerms) -> BcResult<()> {
        if terms.principal() <= Decimal::ZERO {
            return Err(BcError::BadData(
                "loan principal must be greater than zero".into(),
            ));
        }
        if terms.annual_rate() < Decimal::ZERO {
            return Err(BcError::BadData(
                "loan annual rate must not be negative".into(),
            ));
        }
        if terms.term_months() == 0 {
            return Err(BcError::BadData(
                "loan term must be at least one month".into(),
            ));
        }

        // Verify the account is a Receivable.
        let kind: Option<String> = sqlx::query_scalar("SELECT kind FROM accounts WHERE id = ?")
            .bind(terms.account_id().to_string())
            .fetch_optional(&self.pool)
            .await?;

        match kind.as_deref() {
            None => return Err(BcError::NotFound(format!("account {}", terms.account_id()))),
            Some(k) if k != "receivable" => {
                let parsed = crate::db::from_db_str::<bc_models::AccountKind>(k)?;
                return Err(BcError::InvalidAccountKind {
                    operation: "set loan terms",
                    account_id: terms.account_id().clone(),
                    kind: parsed,
                });
            }
            Some(_) => {}
        }

        let id = terms.id().clone();
        let now = jiff::Timestamp::now();

        let freq_str = period_to_db(terms.repayment_frequency())?;
        let compounding_str = compounding_to_db(terms.compounding_frequency())?;

        let event = Event::LoanTermsSet {
            id: id.clone(),
            account_id: terms.account_id().clone(),
            principal: terms.principal(),
            annual_rate: terms.annual_rate(),
            start_date: terms.start_date(),
            term_months: terms.term_months(),
            repayment_frequency: terms.repayment_frequency().clone(),
            commodity: terms.commodity().to_owned(),
        };

        let mut tx = self.pool.begin().await?;
        insert_event(&event, &mut tx).await?;

        sqlx::query(
            "INSERT INTO loan_terms \
             (id, account_id, principal, interest_rate, start_date, term_months, \
              repayment_frequency, commodity, created_at, compounding_frequency) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(terms.account_id().to_string())
        .bind(terms.principal().to_string())
        .bind(terms.annual_rate().to_string())
        .bind(terms.start_date().to_string())
        .bind(i64::from(terms.term_months()))
        .bind(&freq_str)
        .bind(terms.commodity())
        .bind(now.to_string())
        .bind(&compounding_str)
        .execute(&mut *tx)
        .await?;

        for offset_id in terms.offset_account_ids() {
            sqlx::query(
                "INSERT OR IGNORE INTO loan_offset_accounts (loan_id, account_id, created_at) \
                 VALUES (?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(offset_id.to_string())
            .bind(now.to_string())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        tracing::info!(account_id = %terms.account_id(), "loan terms set");
        Ok(())
    }

    /// Returns the most recently set loan terms for `account_id`, or `None`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose loan terms to retrieve.
    ///
    /// # Returns
    ///
    /// The most recent [`LoanTerms`] for the account, or `None` if no terms
    /// have been set.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn loan_terms_for(&self, account_id: &AccountId) -> BcResult<Option<LoanTerms>> {
        let maybe_row: Option<LoanTermsRow> = sqlx::query_as(
            "SELECT id, principal, interest_rate, start_date, term_months, \
             repayment_frequency, commodity, created_at, compounding_frequency \
             FROM loan_terms WHERE account_id = ? \
             ORDER BY rowid DESC LIMIT 1",
        )
        .bind(account_id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = maybe_row else {
            return Ok(None);
        };

        let id = row
            .id
            .parse::<LoanId>()
            .map_err(|e| BcError::BadData(e.to_string()))?;
        let principal = row
            .principal
            .parse::<Decimal>()
            .map_err(|e| BcError::BadData(format!("invalid principal: {e}")))?;
        let annual_rate = row
            .interest_rate
            .parse::<Decimal>()
            .map_err(|e| BcError::BadData(format!("invalid interest_rate: {e}")))?;
        let start_date = row
            .start_date
            .parse::<Date>()
            .map_err(|e| BcError::BadData(format!("invalid start_date: {e}")))?;
        let term_months = u32::try_from(row.term_months)
            .map_err(|e| BcError::BadData(format!("invalid term_months: {e}")))?;
        let repayment_frequency = period_from_db(&row.repayment_frequency)?;
        let compounding_frequency = compounding_from_db(&row.compounding_frequency)?;
        let created_at = row
            .created_at
            .parse::<jiff::Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at: {e}")))?;

        let offset_rows: Vec<(String,)> =
            sqlx::query_as("SELECT account_id FROM loan_offset_accounts WHERE loan_id = ?")
                .bind(row.id.as_str())
                .fetch_all(&self.pool)
                .await?;

        let offset_account_ids = offset_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<AccountId>()
                    .map_err(|e| BcError::BadData(e.to_string()))
            })
            .collect::<BcResult<Vec<_>>>()?;

        Ok(Some(
            LoanTerms::builder()
                .id(id)
                .account_id(account_id.clone())
                .principal(principal)
                .annual_rate(annual_rate)
                .start_date(start_date)
                .term_months(term_months)
                .repayment_frequency(repayment_frequency)
                .compounding_frequency(compounding_frequency)
                .offset_account_ids(offset_account_ids)
                .commodity(row.commodity)
                .created_at(created_at)
                .build(),
        ))
    }

    /// Computes the full amortization schedule for `account_id`.
    ///
    /// `offset_balances` maps each offset account ID to its current balance.
    /// Pass an empty map if there are no offset accounts or balances are unknown.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no loan terms exist for the account.
    /// Returns [`BcError`] on calculation or database failure.
    #[inline]
    pub async fn amortization_schedule(
        &self,
        account_id: &AccountId,
        offset_balances: std::collections::HashMap<AccountId, Decimal>,
    ) -> BcResult<Vec<AmortizationRow>> {
        let terms = self
            .loan_terms_for(account_id)
            .await?
            .ok_or_else(|| BcError::NotFound(format!("loan terms for {account_id}")))?;

        compute_schedule(&terms, &offset_balances)
    }
}

/// Returns the effective total offset balance, clamped to `[0, principal]`.
fn effective_offset(
    offset_ids: &[AccountId],
    balances: &std::collections::HashMap<AccountId, Decimal>,
    principal: Decimal,
) -> Decimal {
    let total: Decimal = offset_ids
        .iter()
        .filter_map(|id| balances.get(id).copied())
        .fold(Decimal::ZERO, Decimal::saturating_add);
    total.min(principal).max(Decimal::ZERO)
}

/// Advances `date` by one repayment period using [`Period::advance`].
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the period type is unsupported for loans.
fn advance_by_period(date: Date, period: &Period) -> BcResult<Date> {
    match period {
        Period::Weekly
        | Period::Fortnightly { .. }
        | Period::Monthly
        | Period::Quarterly
        | Period::FinancialQuarter { .. }
        | Period::FinancialYear { .. }
        | Period::CalendarYear
        | Period::Custom { .. } => Ok(period.advance(date)),
        _ => Err(BcError::BadData(format!(
            "unsupported repayment period for loan: {period:?}"
        ))),
    }
}

/// Returns periods per year for the given repayment period.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the period type is not supported for loans
/// or if a `Custom` period has zero duration.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal division on positive values"
)]
fn periods_per_year(period: &Period) -> BcResult<Decimal> {
    match period {
        Period::Weekly => Ok(Decimal::from(52_u32)),
        Period::Fortnightly { .. } => Ok(Decimal::from(26_u32)),
        Period::Monthly => Ok(Decimal::from(12_u32)),
        Period::Quarterly | Period::FinancialQuarter { .. } => Ok(Decimal::from(4_u32)),
        Period::FinancialYear { .. } | Period::CalendarYear => Ok(Decimal::ONE),
        Period::Custom {
            days,
            weeks,
            months,
        } => {
            let total_days = i64::from(days.unwrap_or(0))
                + i64::from(weeks.unwrap_or(0)) * 7
                + i64::from(months.unwrap_or(0)) * 30;
            if total_days <= 0 {
                return Err(BcError::BadData("custom period has zero duration".into()));
            }
            let d = Decimal::from(total_days);
            Ok(Decimal::from(365_u32) / d)
        }
        _ => Err(BcError::BadData(format!(
            "unsupported repayment period for loan schedule: {period:?}"
        ))),
    }
}

/// Computes the fixed annuity payment amount.
///
/// Uses the formula `P * r * (1+r)^n / ((1+r)^n - 1)`.
/// For zero interest: `P / n`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] on numeric overflow.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal arithmetic on financial values"
)]
fn annuity_payment(principal: Decimal, period_rate: Decimal, n: u32) -> BcResult<Decimal> {
    if n == 0 {
        return Ok(Decimal::ZERO);
    }
    if period_rate == Decimal::ZERO {
        return Ok((principal / Decimal::from(n)).round_dp(2));
    }
    let one_plus_r = Decimal::ONE + period_rate;
    let rate_f64 = one_plus_r
        .to_f64()
        .ok_or_else(|| BcError::BadData(format!("period_rate {period_rate} out of f64 range")))?;
    let compound_f64 = rate_f64.powf(f64::from(n));
    let compound = Decimal::try_from(compound_f64)
        .map_err(|e| BcError::BadData(format!("compound factor out of range: {e}")))?;
    let numerator = principal * period_rate * compound;
    let denominator = compound - Decimal::ONE;
    Ok((numerator / denominator).round_dp(2))
}

/// Returns total number of repayment periods for the loan.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the period type is unsupported or produces too many payments.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal multiplication on positive values"
)]
fn total_payments(term_months: u32, period: &Period) -> BcResult<u32> {
    let ppy = periods_per_year(period)?;
    let n = (Decimal::from(term_months) * ppy / Decimal::from(12_u32)).round_dp(0);
    u32::try_from(n).map_err(|_ignored| {
        BcError::BadData("loan term produces an unreasonably large number of payments".into())
    })
}

/// Computes the amortization schedule from loan terms and optional offset balances.
///
/// For [`CompoundingFrequency::Daily`]: interest = balance × (`annual_rate/365`) × `days_in_period`.
/// For [`CompoundingFrequency::Monthly`]: interest = balance × (`annual_rate/periods_per_year`).
/// In both cases the effective balance is reduced by the sum of `offset_balances`.
///
/// # Errors
///
/// Returns [`BcError`] on calculation failure.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal arithmetic on financial values"
)]
fn compute_schedule(
    terms: &LoanTerms,
    offset_balances: &std::collections::HashMap<AccountId, Decimal>,
) -> BcResult<Vec<AmortizationRow>> {
    let n = total_payments(terms.term_months(), terms.repayment_frequency())?;
    if n == 0 {
        return Ok(vec![]);
    }

    let ppy = periods_per_year(terms.repayment_frequency())?;
    let period_rate = terms.annual_rate() / ppy;
    let offset_total = effective_offset(
        terms.offset_account_ids(),
        offset_balances,
        terms.principal(),
    );

    // Compute fixed payment on the full principal: offset accounts reduce interest
    // charged each period but do NOT reduce the repayment amount itself.
    let payment = annuity_payment(terms.principal(), period_rate, n)?;

    #[expect(
        clippy::as_conversions,
        reason = "n is u32, fits in usize on all platforms"
    )]
    let mut rows = Vec::with_capacity(n as usize);
    let mut balance = terms.principal();
    let mut prev_date = terms.start_date();
    let mut date = advance_by_period(prev_date, terms.repayment_frequency())?;

    for i in 1..=n {
        let effective_balance = (balance - offset_total).max(Decimal::ZERO);

        #[expect(
            clippy::match_same_arms,
            reason = "Monthly and wildcard arms share the same body intentionally; the wildcard handles future non-exhaustive variants"
        )]
        let interest = match terms.compounding_frequency() {
            CompoundingFrequency::Daily => {
                use jiff::Unit;
                let elapsed = prev_date
                    .until((Unit::Day, date))
                    .map_or(0_i64, |s| s.get_days().into())
                    .max(0_i64);
                let days = Decimal::from(elapsed);
                let daily_rate = terms.annual_rate() / Decimal::from(365_u32);
                (effective_balance * daily_rate * days).round_dp(2)
            }
            CompoundingFrequency::Monthly => (effective_balance * period_rate).round_dp(2),
            _ => (effective_balance * period_rate).round_dp(2),
        };

        let is_last = i == n;
        let principal_portion = if is_last {
            balance
        } else {
            (payment - interest).max(Decimal::ZERO)
        };

        balance -= principal_portion;
        if balance.abs() < Decimal::new(5, 3) {
            balance = Decimal::ZERO;
        }

        rows.push(AmortizationRow::new(
            i,
            date,
            interest + principal_portion,
            principal_portion,
            interest,
            balance,
        ));

        if !is_last {
            prev_date = date;
            date = advance_by_period(date, terms.repayment_frequency())?;
        }
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::LoanTerms;
    use bc_models::Period;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    async fn make_receivable(pool: &SqlitePool) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name("Loan to Friend")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Receivable)
            .call()
            .await
            .expect("create Receivable account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_and_retrieve_loan_terms(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());

        let terms = LoanTerms::builder()
            .account_id(account_id.clone())
            .principal(dec!(100_000))
            .annual_rate(dec!(0.065))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(360_u32)
            .repayment_frequency(Period::Monthly)
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let retrieved = svc
            .loan_terms_for(&account_id)
            .await
            .expect("get terms")
            .expect("terms should exist");

        assert_eq!(retrieved.principal(), dec!(100_000));
        assert_eq!(retrieved.annual_rate(), dec!(0.065));
        assert_eq!(retrieved.term_months(), 360);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amortization_schedule_first_payment_splits_correctly(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());

        let terms = LoanTerms::builder()
            .account_id(account_id.clone())
            .principal(dec!(100_000))
            .annual_rate(dec!(0.06)) // 6% p.a. = 0.5% per month
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(120_u32)
            .repayment_frequency(Period::Monthly)
            .compounding_frequency(bc_models::CompoundingFrequency::Monthly)
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let schedule = svc
            .amortization_schedule(&account_id, std::collections::HashMap::new())
            .await
            .expect("schedule");
        assert_eq!(schedule.len(), 120);

        let first = schedule.first().expect("first payment");
        assert_eq!(first.payment_number, 1);
        // Interest for month 1 = 100_000 * 0.005 = 500.00
        assert_eq!(first.interest, dec!(500.00));
        // Total balance after last payment should be ~0
        let last = schedule.last().expect("last payment");
        assert!(
            last.remaining_balance.abs() < dec!(0.10),
            "balance should be near zero, got {}",
            last.remaining_balance
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loan_terms_for_returns_none_when_not_set(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());
        let result = svc.loan_terms_for(&account_id).await.expect("query");
        assert!(result.is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amortization_schedule_custom_28_day_period(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());

        let terms = LoanTerms::builder()
            .account_id(account_id.clone())
            .principal(dec!(10_000))
            .annual_rate(dec!(0.06))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Custom {
                days: Some(28),
                weeks: None,
                months: None,
            })
            .compounding_frequency(bc_models::CompoundingFrequency::Monthly)
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let schedule = svc
            .amortization_schedule(&account_id, std::collections::HashMap::new())
            .await
            .expect("schedule");
        // ~13 payments in a year for 28-day periods
        assert!(!schedule.is_empty(), "schedule should be non-empty");
        let last = schedule.last().expect("last payment");
        assert!(
            last.remaining_balance.abs() < dec!(1.00),
            "balance should be near zero, got {}",
            last.remaining_balance
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_loan_terms_on_deposit_account_fails(pool: SqlitePool) {
        let deposit_id = crate::AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create DepositAccount");
        let svc = super::Service::new(pool.clone());
        let terms = LoanTerms::builder()
            .account_id(deposit_id.clone())
            .principal(dec!(10_000))
            .annual_rate(dec!(0.05))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Monthly)
            .commodity("AUD")
            .build();
        let result = svc.set_loan_terms(&terms).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidAccountKind { .. })),
            "expected InvalidAccountKind, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn daily_accrual_interest_greater_than_monthly(pool: SqlitePool) {
        use bc_models::CompoundingFrequency;
        use bc_models::Period;

        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());

        let terms = LoanTerms::builder()
            .account_id(account_id.clone())
            .principal(dec!(100_000))
            .annual_rate(dec!(0.06))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Monthly)
            .compounding_frequency(CompoundingFrequency::Daily)
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let schedule = svc
            .amortization_schedule(&account_id, std::collections::HashMap::new())
            .await
            .expect("schedule");
        pretty_assertions::assert_eq!(schedule.len(), 12);

        // First month (Jan 2026) has 31 days.
        // daily_rate = 0.06 / 365 ≈ 0.000164384
        // interest = 100_000 * 0.000164384 * 31 ≈ 509.59
        let first = schedule.first().expect("first payment");
        assert!(
            first.interest > dec!(509) && first.interest < dec!(511),
            "expected ~509.59 interest for Jan (31 days), got {}",
            first.interest
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_loan_terms_rejects_zero_principal(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());
        let terms = LoanTerms::builder()
            .account_id(account_id)
            .principal(rust_decimal::Decimal::ZERO)
            .annual_rate(dec!(0.05))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Monthly)
            .commodity("AUD")
            .build();
        let result = svc.set_loan_terms(&terms).await;
        assert!(
            matches!(result, Err(crate::BcError::BadData(_))),
            "expected BadData for zero principal, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_loan_terms_rejects_negative_annual_rate(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());
        let terms = LoanTerms::builder()
            .account_id(account_id)
            .principal(dec!(10_000))
            .annual_rate(rust_decimal::Decimal::new(-1, 2))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Monthly)
            .commodity("AUD")
            .build();
        let result = svc.set_loan_terms(&terms).await;
        assert!(
            matches!(result, Err(crate::BcError::BadData(_))),
            "expected BadData for negative annual rate, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_loan_terms_rejects_zero_term(pool: SqlitePool) {
        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());
        let terms = LoanTerms::builder()
            .account_id(account_id)
            .principal(dec!(10_000))
            .annual_rate(dec!(0.05))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(0_u32)
            .repayment_frequency(Period::Monthly)
            .commodity("AUD")
            .build();
        let result = svc.set_loan_terms(&terms).await;
        assert!(
            matches!(result, Err(crate::BcError::BadData(_))),
            "expected BadData for zero term_months, got {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn offset_account_reduces_interest(pool: SqlitePool) {
        use std::collections::HashMap;

        use bc_models::CompoundingFrequency;
        use bc_models::Period;

        let account_id = make_receivable(&pool).await;
        let svc = super::Service::new(pool.clone());

        let offset_id = crate::AccountService::new(pool.clone())
            .create()
            .name("Offset Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create offset account");

        let terms = LoanTerms::builder()
            .account_id(account_id.clone())
            .principal(dec!(500_000))
            .annual_rate(dec!(0.06))
            .start_date(jiff::civil::date(2026, 1, 1))
            .term_months(12_u32)
            .repayment_frequency(Period::Monthly)
            .compounding_frequency(CompoundingFrequency::Daily)
            .offset_account_ids(vec![offset_id.clone()])
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let no_offset = svc
            .amortization_schedule(&account_id, HashMap::new())
            .await
            .expect("schedule without offset");

        let offset_balances = HashMap::from([(offset_id, dec!(300_000))]);
        let with_offset = svc
            .amortization_schedule(&account_id, offset_balances)
            .await
            .expect("schedule with offset");

        let with_first = with_offset
            .first()
            .expect("schedule with offset has payments");
        let no_first = no_offset
            .first()
            .expect("schedule without offset has payments");

        // Offset reduces interest but must NOT change the fixed repayment amount.
        pretty_assertions::assert_eq!(
            with_first.total_payment,
            no_first.total_payment,
            "offset must not change the fixed payment amount"
        );
        assert!(
            with_first.interest < no_first.interest,
            "offset should reduce interest: {} vs {}",
            with_first.interest,
            no_first.interest
        );
    }
}
