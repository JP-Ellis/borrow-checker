//! Loan terms persistence and amortization schedule calculation.

use bc_models::AccountId;
use bc_models::AmortizationRow;
use bc_models::LoanId;
use bc_models::LoanTerms;
use bc_models::RepaymentFrequency;
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
    /// Serialised [`RepaymentFrequency`] — unit variants or `"custom:<days>"`.
    repayment_frequency: String,
    /// ISO 4217 commodity code (e.g. `"AUD"`).
    commodity: String,
    /// ISO 8601 timestamp string for when the record was created.
    created_at: String,
}

/// Serialises a [`RepaymentFrequency`] to a DB string.
///
/// Unit variants use `to_db_str`; the `Custom` variant is stored as `"custom:<period_days>"`.
///
/// # Errors
///
/// Returns [`BcError`] if the unit-variant serde output is not a plain string.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "Frequency is #[non_exhaustive]; new unit variants should serialize via the default path"
)]
fn freq_to_db(freq: RepaymentFrequency) -> BcResult<String> {
    match freq {
        RepaymentFrequency::Custom { period_days } => Ok(format!("custom:{period_days}")),
        other => crate::db::to_db_str(other),
    }
}

/// Deserialises a [`RepaymentFrequency`] from a DB string.
///
/// Handles both unit-variant strings and the `"custom:<period_days>"` format.
///
/// # Errors
///
/// Returns [`BcError`] if the string cannot be parsed.
fn freq_from_db(s: &str) -> BcResult<RepaymentFrequency> {
    if let Some(days_str) = s.strip_prefix("custom:") {
        let period_days = days_str.parse::<u32>().map_err(|e| {
            BcError::BadData(format!("invalid custom period_days '{days_str}': {e}"))
        })?;
        if period_days == 0 {
            return Err(BcError::BadData(
                "custom repayment period_days must be at least 1".into(),
            ));
        }
        return Ok(RepaymentFrequency::Custom { period_days });
    }
    crate::db::from_db_str(s)
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
    /// projection row.
    ///
    /// # Arguments
    ///
    /// * `terms` - The loan terms to persist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::InvalidAccountKind`] if the account is not a `Receivable`.
    /// Returns [`BcError::BadData`] if `Custom` frequency has `period_days == 0`.
    /// Returns [`BcError`] on serialisation or database failure.
    #[inline]
    pub async fn set_loan_terms(&self, terms: &LoanTerms) -> BcResult<()> {
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

        if let RepaymentFrequency::Custom { period_days } = terms.repayment_frequency() {
            if period_days == 0 {
                return Err(BcError::BadData(
                    "custom repayment period_days must be at least 1".into(),
                ));
            }
        }

        let id = terms.id().clone();
        let now = jiff::Timestamp::now();

        let freq_str = freq_to_db(terms.repayment_frequency())?;

        let event = Event::LoanTermsSet {
            id: id.clone(),
            account_id: terms.account_id().clone(),
            principal: terms.principal(),
            annual_rate: terms.annual_rate(),
            start_date: terms.start_date(),
            term_months: terms.term_months(),
            repayment_frequency: terms.repayment_frequency(),
            commodity: terms.commodity().to_owned(),
        };

        let mut tx = self.pool.begin().await?;
        insert_event(&event, &mut tx).await?;

        sqlx::query(
            "INSERT INTO loan_terms \
             (id, account_id, principal, interest_rate, start_date, term_months, repayment_frequency, commodity, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
        .execute(&mut *tx)
        .await?;

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
             repayment_frequency, commodity, created_at \
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
        let repayment_frequency = freq_from_db(&row.repayment_frequency)?;
        let created_at = row
            .created_at
            .parse::<jiff::Timestamp>()
            .map_err(|e| BcError::BadData(format!("invalid created_at: {e}")))?;

        Ok(Some(
            LoanTerms::builder()
                .id(id)
                .account_id(account_id.clone())
                .principal(principal)
                .annual_rate(annual_rate)
                .start_date(start_date)
                .term_months(term_months)
                .repayment_frequency(repayment_frequency)
                .commodity(row.commodity)
                .created_at(created_at)
                .build(),
        ))
    }

    /// Computes the full amortization schedule for `account_id`.
    ///
    /// Returns an error if no loan terms have been set for the account.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose amortization schedule to compute.
    ///
    /// # Returns
    ///
    /// A vector of [`AmortizationRow`] entries, one per payment period.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no loan terms exist for the account.
    /// Returns [`BcError`] on calculation or database failure.
    #[inline]
    pub async fn amortization_schedule(
        &self,
        account_id: &AccountId,
    ) -> BcResult<Vec<AmortizationRow>> {
        let terms = self
            .loan_terms_for(account_id)
            .await?
            .ok_or_else(|| BcError::NotFound(format!("loan terms for {account_id}")))?;

        compute_schedule(&terms)
    }
}

/// Returns the number of days to advance per period for `frequency`.
///
/// For [`RepaymentFrequency::Weekly`], [`RepaymentFrequency::Fortnightly`], and
/// [`RepaymentFrequency::Custom`], returns the fixed day count. For calendar-month-based
/// frequencies, returns `None` to indicate that [`months_per_period`] should be used instead.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RepaymentFrequency is #[non_exhaustive]; calendar-month arithmetic is the safe fallback for new variants"
)]
fn days_per_period(frequency: RepaymentFrequency) -> Option<i64> {
    match frequency {
        RepaymentFrequency::Weekly => Some(7),
        RepaymentFrequency::Fortnightly => Some(14),
        RepaymentFrequency::Custom { period_days } => Some(i64::from(period_days)),
        _ => None,
    }
}

/// Returns calendar months to advance per period.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `frequency` does not use calendar-month advancement.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RepaymentFrequency is #[non_exhaustive]; unrecognised variants are an error not a fallback"
)]
fn months_per_period(frequency: RepaymentFrequency) -> BcResult<i32> {
    match frequency {
        RepaymentFrequency::Monthly => Ok(1),
        RepaymentFrequency::Quarterly => Ok(3),
        other => Err(BcError::BadData(format!(
            "frequency {other:?} does not use calendar-month advancement"
        ))),
    }
}

/// Advances `date` by one payment period for the given `frequency`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if date arithmetic overflows or the frequency is unrecognised.
fn advance_date(date: Date, frequency: RepaymentFrequency) -> BcResult<Date> {
    if let Some(days) = days_per_period(frequency) {
        return date
            .checked_add(jiff::Span::new().days(days))
            .map_err(|_e| BcError::BadData("date arithmetic overflow advancing by days".into()));
    }
    let months = months_per_period(frequency)?;
    date.checked_add(jiff::Span::new().months(months))
        .map_err(|_e| BcError::BadData("date arithmetic overflow advancing by months".into()))
}

/// Computes the total number of payment periods for the given loan terms.
///
/// For standard frequencies, the number of periods is `term_months * periods_per_year / 12`.
/// For `Custom`, it is approximated as `term_months * (365.25 / 12) / period_days`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the computed number of payments exceeds [`u32::MAX`]
/// (i.e. a loan term of ~11.7 million years at weekly frequency).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "Decimal arithmetic on small loan-term values; overflow is not possible in practice"
)]
fn total_payments(terms: &LoanTerms) -> BcResult<u32> {
    let freq = terms.repayment_frequency();
    let periods_per_year = freq.periods_per_year();
    let term_months = Decimal::from(terms.term_months());
    let n = (term_months * periods_per_year / Decimal::from(12_u32)).round_dp(0);
    n.try_into().map_err(|_e| {
        BcError::BadData("loan term produces an unreasonably large number of payments".into())
    })
}

/// Computes a standard annuity amortization schedule from `terms`.
///
/// Uses the formula:
/// - `period_rate = annual_rate / periods_per_year`
/// - `n` = total payments
/// - `payment = principal * period_rate * (1+r)^n / ((1+r)^n - 1)`
/// - For each period: `interest = balance * r`, `principal = payment - interest`
/// - Last payment: adjusted to clear any rounding residual
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the period rate or compound factor cannot be
/// represented in the required numeric types.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "all arithmetic is on Decimal values in the context of financial calculations where overflow is not a practical concern"
)]
fn compute_schedule(terms: &LoanTerms) -> BcResult<Vec<AmortizationRow>> {
    let freq = terms.repayment_frequency();
    let n = total_payments(terms)?;
    if n == 0 {
        return Ok(vec![]);
    }

    let annual_rate = terms.annual_rate();
    let periods_per_year = freq.periods_per_year();
    let period_rate = annual_rate / periods_per_year;

    let principal = terms.principal();

    // Compute regular payment amount using annuity formula.
    // payment = P * r * (1+r)^n / ((1+r)^n - 1)
    // Handle zero interest rate as edge case.
    let payment = if period_rate == Decimal::ZERO {
        // Zero-interest: equal principal payments.
        let n_dec = Decimal::from(n);
        (principal / n_dec).round_dp(2)
    } else {
        let one_plus_r = Decimal::ONE + period_rate;
        // Use f64 for the power computation, then convert back to Decimal.
        let rate_f64 = one_plus_r.to_f64().ok_or_else(|| {
            BcError::BadData(format!("period_rate {period_rate} out of f64 range"))
        })?;
        let n_f64 = f64::from(n);
        let compound_f64 = rate_f64.powf(n_f64);
        let compound = Decimal::try_from(compound_f64)
            .map_err(|e| BcError::BadData(format!("compound factor out of Decimal range: {e}")))?;
        // P * r * compound / (compound - 1)
        let numerator = principal * period_rate * compound;
        let denominator = compound - Decimal::ONE;
        (numerator / denominator).round_dp(2)
    };

    #[expect(
        clippy::as_conversions,
        reason = "n is a u32 derived from term_months which fits safely into usize on all supported platforms"
    )]
    let mut rows = Vec::with_capacity(n as usize);
    let mut balance = principal;
    let mut date = terms.start_date();

    // Advance to first payment date.
    date = advance_date(date, freq)?;

    for i in 1..=n {
        let interest = (balance * period_rate).round_dp(2);
        let mut principal_portion = payment - interest;
        let is_last = i == n;

        if is_last {
            // On the final payment, use exact remaining balance to clear rounding residual.
            principal_portion = balance;
        }

        balance -= principal_portion;

        // Clamp near-zero balance (abs < 0.005) to exactly zero.
        if balance.abs() < Decimal::new(5, 3) {
            balance = Decimal::ZERO;
        }

        let total = interest + principal_portion;

        rows.push(AmortizationRow::new(
            i,
            date,
            total,
            principal_portion,
            interest,
            balance,
        ));

        if !is_last {
            date = advance_date(date, freq)?;
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
    use bc_models::RepaymentFrequency;
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

    #[test]
    fn months_per_period_errors_on_unrecognised_variant() {
        // Monthly should return Ok(1)
        use bc_models::RepaymentFrequency;
        pretty_assertions::assert_eq!(
            super::months_per_period(RepaymentFrequency::Monthly).unwrap(),
            1
        );
        // Quarterly should return Ok(3)
        pretty_assertions::assert_eq!(
            super::months_per_period(RepaymentFrequency::Quarterly).unwrap(),
            3
        );
        assert!(
            super::months_per_period(RepaymentFrequency::Weekly).is_err(),
            "Weekly is not a calendar-month frequency"
        );
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
            .repayment_frequency(RepaymentFrequency::Monthly)
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
            .repayment_frequency(RepaymentFrequency::Monthly)
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let schedule = svc
            .amortization_schedule(&account_id)
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
            .repayment_frequency(RepaymentFrequency::Custom { period_days: 28 })
            .commodity("AUD")
            .build();

        svc.set_loan_terms(&terms).await.expect("set terms");

        let schedule = svc
            .amortization_schedule(&account_id)
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
            .repayment_frequency(RepaymentFrequency::Monthly)
            .commodity("AUD")
            .build();
        let result = svc.set_loan_terms(&terms).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidAccountKind { .. })),
            "expected InvalidAccountKind, got {result:?}"
        );
    }
}
