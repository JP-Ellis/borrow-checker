//! Advisory warnings raised by write paths that accept their input anyway.
//!
//! The project's governing principle is "warn, don't block": guardrails inform
//! rather than gatekeep, and hard errors are reserved for genuinely
//! unrepresentable states. A [`Warning`] is what that principle produces — the
//! write happened, and something about it is worth saying.

use std::collections::HashMap;

use bc_models::AccountId;
use jiff::civil::Date;

/// Something worth telling the user about a write that nonetheless succeeded.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum Warning {
    /// A posting's commodity is outside its account's non-empty declared list.
    ///
    /// Compared by code, not by id: a [`bc_models::Amount`] carries a
    /// [`bc_models::CommodityCode`] and no id, so the account's declared ids are
    /// resolved to codes to compare. `commodities.code` is not unique across
    /// exchanges, so an account declaring one exchange's BTC accepts a posting
    /// coded `BTC` from any exchange. The posting carries nothing finer, so no
    /// stricter comparison is available.
    CommodityOutsideAccountList {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The commodity code the posting used.
        commodity_code: String,
    },
    /// A transaction dated before its account's declared opening date.
    PostingBeforeAccountOpened {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The transaction's value date.
        date: Date,
        /// The account's declared opening date.
        opened_on: Date,
    },
    /// A transaction dated after its account's declared closing date.
    PostingAfterAccountClosed {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
        /// The transaction's value date.
        date: Date,
        /// The account's declared closing date.
        closed_on: Date,
    },
    /// A posting written into an archived account.
    PostingIntoArchivedAccount {
        /// The account holding the posting.
        account_id: AccountId,
        /// The account's colon-joined path, for display.
        account_path: String,
    },
}

impl std::fmt::Display for Warning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::CommodityOutsideAccountList {
                ref account_path,
                ref commodity_code,
                ..
            } => write!(
                f,
                "{account_path} does not declare {commodity_code} among the commodities it holds"
            ),
            Self::PostingBeforeAccountOpened {
                ref account_path,
                date,
                opened_on,
                ..
            } => write!(
                f,
                "{account_path} is dated {date} but the account opened on {opened_on}"
            ),
            Self::PostingAfterAccountClosed {
                ref account_path,
                date,
                closed_on,
                ..
            } => write!(
                f,
                "{account_path} is dated {date} but the account closed on {closed_on}"
            ),
            Self::PostingIntoArchivedAccount {
                ref account_path, ..
            } => write!(f, "{account_path} is archived"),
        }
    }
}

/// A value paired with the warnings raised while producing it.
///
/// The write succeeded either way; `warnings` is advisory.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_structs,
    reason = "stable two-field carrier, exhaustively destructured by bc-cli and bc-app"
)]
pub struct Warned<T> {
    /// The value produced.
    pub value: T,
    /// Warnings raised while producing it. Empty is the common case.
    pub warnings: Vec<Warning>,
}

impl<T> Warned<T> {
    /// Pairs a value with its warnings.
    #[inline]
    #[must_use]
    pub const fn new(value: T, warnings: Vec<Warning>) -> Self {
        Self { value, warnings }
    }

    /// Wraps a value that raised no warnings.
    #[inline]
    #[must_use]
    pub const fn clean(value: T) -> Self {
        Self {
            value,
            warnings: Vec::new(),
        }
    }

    /// Discards the warnings and returns the value.
    #[inline]
    #[must_use]
    pub fn into_inner(self) -> T {
        self.value
    }
}

/// The per-account facts the guard needs, fetched once per distinct account.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the transaction write path in Task 6; only exercised by tests until then"
    )
)]
struct Guard {
    /// The account's colon-joined path, for display.
    path: String,
    /// Whether the account is archived.
    archived: bool,
    /// The account's declared opening date, if any.
    opened_on: Option<Date>,
    /// The account's declared closing date, if any.
    closed_on: Option<Date>,
    /// Allowed commodity codes, resolved from the account's declared ids.
    /// Empty means unrestricted.
    commodity_codes: Vec<String>,
}

/// Checks a transaction's postings against their accounts, returning advisory
/// warnings. Never fails a write: every finding is a [`Warning`].
///
/// `date` is the transaction's value date. Postings carry no date of their own,
/// so it is the only business date available to compare against an account's
/// declared life. Both bounds are inclusive.
///
/// # Arguments
///
/// * `conn` - An open connection or transaction to read accounts through.
/// * `date` - The transaction's value date.
/// * `postings` - The postings to check.
///
/// # Errors
///
/// Returns [`crate::BcError`] only on database read failure. A posting whose
/// account does not exist yields no warning — referential integrity is not
/// this function's job.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the transaction write path in Task 6; only exercised by tests until then"
    )
)]
pub(crate) async fn check_postings(
    conn: &mut sqlx::SqliteConnection,
    date: Date,
    postings: &[bc_models::Posting],
) -> crate::BcResult<Vec<Warning>> {
    let mut guards: HashMap<AccountId, Guard> = HashMap::new();
    let mut warnings = Vec::new();

    for posting in postings {
        let account_id = posting.account_id().clone();
        if !guards.contains_key(&account_id) {
            let Some(guard) = load_guard(&mut *conn, &account_id).await? else {
                continue;
            };
            guards.insert(account_id.clone(), guard);
        }
        let Some(guard) = guards.get(&account_id) else {
            continue;
        };

        if guard.archived {
            warnings.push(Warning::PostingIntoArchivedAccount {
                account_id: account_id.clone(),
                account_path: guard.path.clone(),
            });
        }

        if let Some(opened_on) = guard.opened_on
            && date < opened_on
        {
            warnings.push(Warning::PostingBeforeAccountOpened {
                account_id: account_id.clone(),
                account_path: guard.path.clone(),
                date,
                opened_on,
            });
        }

        if let Some(closed_on) = guard.closed_on
            && date > closed_on
        {
            warnings.push(Warning::PostingAfterAccountClosed {
                account_id: account_id.clone(),
                account_path: guard.path.clone(),
                date,
                closed_on,
            });
        }

        if !guard.commodity_codes.is_empty()
            && let Some(amount) = posting.amount()
        {
            let code = amount.commodity().as_str();
            if !guard.commodity_codes.iter().any(|c| c == code) {
                warnings.push(Warning::CommodityOutsideAccountList {
                    account_id: account_id.clone(),
                    account_path: guard.path.clone(),
                    commodity_code: code.to_owned(),
                });
            }
        }
    }

    Ok(warnings)
}

/// Raw `accounts` columns needed to build a [`Guard`]: name, `parent_id`,
/// `archived_at`, `opened_on`, `closed_on`.
type GuardRow = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// Fetches one account's guard facts, or `None` if no such account exists.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the transaction write path in Task 6; only exercised by tests until then"
    )
)]
async fn load_guard(
    conn: &mut sqlx::SqliteConnection,
    id: &AccountId,
) -> crate::BcResult<Option<Guard>> {
    let row: Option<GuardRow> = sqlx::query_as(
        "SELECT name, parent_id, archived_at, opened_on, closed_on \
             FROM accounts WHERE id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *conn)
    .await?;

    let Some((name, parent_id, archived_at, opened_on, closed_on)) = row else {
        return Ok(None);
    };

    let mut segments = vec![name];
    let mut cursor = parent_id;
    while let Some(parent) = cursor {
        let next: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT name, parent_id FROM accounts WHERE id = ?")
                .bind(&parent)
                .fetch_optional(&mut *conn)
                .await?;
        let Some((parent_name, grandparent)) = next else {
            break;
        };
        segments.push(parent_name);
        cursor = grandparent;
    }
    segments.reverse();

    Ok(Some(Guard {
        path: segments.join(":"),
        archived: archived_at.is_some(),
        opened_on: parse_date_column(opened_on.as_deref(), "opened_on")?,
        closed_on: parse_date_column(closed_on.as_deref(), "closed_on")?,
        commodity_codes: load_allowed_codes(conn, id).await?,
    }))
}

/// Parses a nullable `YYYY-MM-DD` column, naming it in any error.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the transaction write path in Task 6; only exercised by tests until then"
    )
)]
fn parse_date_column(raw: Option<&str>, column: &str) -> crate::BcResult<Option<Date>> {
    raw.map(|s| {
        s.parse::<Date>()
            .map_err(|e| crate::BcError::BadData(format!("invalid {column} '{s}': {e}")))
    })
    .transpose()
}

/// Fetches the codes of an account's declared commodities, in declaration order.
///
/// Joins through to `commodities` because `account_commodities` stores ids
/// while a posting's [`bc_models::Amount`] carries only a code.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "wired into the transaction write path in Task 6; only exercised by tests until then"
    )
)]
async fn load_allowed_codes(
    conn: &mut sqlx::SqliteConnection,
    id: &AccountId,
) -> crate::BcResult<Vec<String>> {
    sqlx::query_scalar(
        "SELECT c.code FROM account_commodities ac \
         JOIN commodities c ON c.id = ac.commodity_id \
         WHERE ac.account_id = ? ORDER BY ac.position",
    )
    .bind(id.to_string())
    .fetch_all(&mut *conn)
    .await
    .map_err(crate::BcError::from)
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::Warned;
    use super::Warning;
    use super::check_postings;

    /// Registers AUD and BTC, returning their ids in that order.
    async fn seed_commodities(
        pool: &sqlx::SqlitePool,
    ) -> (bc_models::CommodityId, bc_models::CommodityId) {
        let svc = crate::commodity::Service::new(pool.clone());
        let aud = svc
            .create(
                &bc_models::Commodity::builder()
                    .code("AUD")
                    .name("Australian Dollar")
                    .decimals(2_u8)
                    .is_iso(true)
                    .symbol_after(false)
                    .build(),
            )
            .await
            .expect("register AUD");
        let btc = svc
            .create(
                &bc_models::Commodity::builder()
                    .code("BTC")
                    .name("Bitcoin")
                    .decimals(8_u8)
                    .is_iso(false)
                    .symbol_after(false)
                    .build(),
            )
            .await
            .expect("register BTC");
        (aud.id().clone(), btc.id().clone())
    }

    /// A single-leg posting list against `account` in `code`.
    fn one_leg(account: bc_models::AccountId, code: &str) -> Vec<Posting> {
        vec![
            Posting::builder()
                .id(PostingId::new())
                .account_id(account)
                .amount(Amount::new(dec!(50.00), CommodityCode::new(code)))
                .build(),
        ]
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn commodity_outside_a_declared_list_warns(pool: sqlx::SqlitePool) {
        let (aud, _btc) = seed_commodities(&pool).await;
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .commodity_ids(&[aud])
            .call()
            .await
            .expect("create account");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2022, 3, 3), &one_leg(id, "BTC"))
            .await
            .expect("check postings");

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            matches!(
                warnings[0],
                Warning::CommodityOutsideAccountList { ref commodity_code, .. }
                    if commodity_code == "BTC"
            ),
            "{warnings:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn commodity_inside_a_declared_list_is_silent(pool: sqlx::SqlitePool) {
        let (aud, _btc) = seed_commodities(&pool).await;
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .commodity_ids(&[aud])
            .call()
            .await
            .expect("create account");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2022, 3, 3), &one_leg(id, "AUD"))
            .await
            .expect("check postings");

        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_empty_commodity_list_is_unrestricted(pool: sqlx::SqlitePool) {
        seed_commodities(&pool).await;
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2022, 3, 3), &one_leg(id, "BTC"))
            .await
            .expect("check postings");

        assert!(
            warnings.is_empty(),
            "an empty list must not restrict: {warnings:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn a_date_before_opened_on_warns(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .opened_on(date(2020, 1, 1))
            .call()
            .await
            .expect("create account");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2019, 5, 1), &one_leg(id, "AUD"))
            .await
            .expect("check postings");

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            matches!(
                warnings[0],
                Warning::PostingBeforeAccountOpened { date: d, opened_on, .. }
                    if d == date(2019, 5, 1) && opened_on == date(2020, 1, 1)
            ),
            "{warnings:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn a_date_after_closed_on_warns(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        sqlx::query("UPDATE accounts SET closed_on = ?1 WHERE id = ?2")
            .bind("2024-06-30")
            .bind(id.to_string())
            .execute(&pool)
            .await
            .expect("seed closed_on");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2025, 1, 15), &one_leg(id, "AUD"))
            .await
            .expect("check postings");

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            matches!(warnings[0], Warning::PostingAfterAccountClosed { .. }),
            "{warnings:?}"
        );
    }

    /// Both bounds are inclusive, so a transaction dated exactly on either one
    /// sits inside the account's life.
    async fn assert_date_inside_life_is_silent(pool: sqlx::SqlitePool, when: jiff::civil::Date) {
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .opened_on(date(2020, 1, 1))
            .call()
            .await
            .expect("create account");
        sqlx::query("UPDATE accounts SET closed_on = ?1 WHERE id = ?2")
            .bind("2024-06-30")
            .bind(id.to_string())
            .execute(&pool)
            .await
            .expect("seed closed_on");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, when, &one_leg(id, "AUD"))
            .await
            .expect("check postings");

        assert!(
            warnings.is_empty(),
            "{when} must be inside the life: {warnings:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_date_on_opened_on_is_silent(pool: sqlx::SqlitePool) {
        assert_date_inside_life_is_silent(pool, date(2020, 1, 1)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_date_mid_life_is_silent(pool: sqlx::SqlitePool) {
        assert_date_inside_life_is_silent(pool, date(2022, 3, 3)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_date_on_closed_on_is_silent(pool: sqlx::SqlitePool) {
        assert_date_inside_life_is_silent(pool, date(2024, 6, 30)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn an_archived_account_warns(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let id = accounts
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
        sqlx::query("UPDATE accounts SET archived_at = ?1 WHERE id = ?2")
            .bind(jiff::Timestamp::now().to_string())
            .bind(id.to_string())
            .execute(&pool)
            .await
            .expect("seed archived_at");

        let mut conn = pool.acquire().await.expect("acquire");
        let warnings = check_postings(&mut conn, date(2022, 3, 3), &one_leg(id, "AUD"))
            .await
            .expect("check postings");

        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            matches!(warnings[0], Warning::PostingIntoArchivedAccount { .. }),
            "{warnings:?}"
        );
    }

    #[test]
    fn clean_carries_no_warnings() {
        let warned = Warned::clean(7_u32);
        assert_eq!(warned.value, 7);
        assert!(warned.warnings.is_empty());
    }

    #[test]
    fn into_inner_discards_warnings() {
        let warned = Warned::new(
            7_u32,
            vec![Warning::PostingIntoArchivedAccount {
                account_id: bc_models::AccountId::new(),
                account_path: "Assets:BankA:Checking".to_owned(),
            }],
        );
        assert_eq!(warned.into_inner(), 7);
    }

    #[test]
    fn warning_display_names_the_account_and_the_dates() {
        let warning = Warning::PostingBeforeAccountOpened {
            account_id: bc_models::AccountId::new(),
            account_path: "Assets:BankA:Checking".to_owned(),
            date: date(2019, 5, 1),
            opened_on: date(2020, 1, 1),
        };
        let rendered = warning.to_string();
        assert!(rendered.contains("Assets:BankA:Checking"), "{rendered}");
        assert!(rendered.contains("2019-05-01"), "{rendered}");
        assert!(rendered.contains("2020-01-01"), "{rendered}");
    }
}
