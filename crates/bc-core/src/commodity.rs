//! Persisted commodity/currency registry: canonical code, display symbol, and
//! alternate input markers (aliases).

use std::collections::HashMap;

use bc_models::Commodity;
use bc_models::CommodityId;
use sqlx::Row as _;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// Default currencies seeded into a fresh database: (code, symbol, name, aliases).
/// Alias sets are deliberately unambiguous (no marker maps to two currencies).
const DEFAULT_CURRENCIES: &[(&str, &str, &str, &[&str])] = &[
    ("USD", "$", "US Dollar", &["US$"]),
    ("AUD", "A$", "Australian Dollar", &["AU$"]),
    ("EUR", "€", "Euro", &[]),
    ("GBP", "£", "British Pound", &[]),
    ("JPY", "¥", "Japanese Yen", &[]),
    ("KRW", "₩", "Korean Won", &[]),
    ("INR", "₹", "Indian Rupee", &[]),
    ("BTC", "₿", "Bitcoin", &[]),
    ("ETH", "ETH", "Ethereum", &[]),
];

/// Read/write access to the commodity registry.
#[derive(Debug, Clone)]
pub struct Service {
    /// Shared SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a service over `pool`.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Inserts a commodity and its aliases in one transaction.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    pub async fn register(&self, c: &Commodity) -> BcResult<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO commodities (id, code, exchange, name, description, symbol, active_from, active_until) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(c.id().to_string())
        .bind(c.code())
        .bind(c.exchange())
        .bind(c.name())
        .bind(c.description())
        .bind(c.symbol())
        .bind(c.active_from().map(|d| d.to_string()))
        .bind(c.active_until().map(|d| d.to_string()))
        .execute(&mut *tx)
        .await?;
        for (position, alias) in c.aliases().iter().enumerate() {
            sqlx::query(
                "INSERT INTO commodity_aliases (commodity_id, alias, position) VALUES (?, ?, ?)",
            )
            .bind(c.id().to_string())
            .bind(alias)
            .bind(
                i64::try_from(position)
                    .map_err(|e| BcError::BadData(format!("alias position overflow: {e}")))?,
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Returns all commodities with their aliases populated.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure or an unparsable stored id.
    pub async fn list_active(&self) -> BcResult<Vec<Commodity>> {
        let rows = sqlx::query(
            "SELECT id, code, exchange, name, description, symbol, active_from, active_until \
             FROM commodities ORDER BY code ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        let alias_rows = sqlx::query(
            "SELECT commodity_id, alias FROM commodity_aliases ORDER BY commodity_id, position",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        for r in &alias_rows {
            aliases
                .entry(r.get::<String, _>("commodity_id"))
                .or_default()
                .push(r.get::<String, _>("alias"));
        }

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id_str: String = r.get("id");
            let id = id_str
                .parse::<CommodityId>()
                .map_err(|e| BcError::BadData(format!("invalid commodity id '{id_str}': {e}")))?;
            let active_from = parse_opt_date(r.get::<Option<String>, _>("active_from"))?;
            let active_until = parse_opt_date(r.get::<Option<String>, _>("active_until"))?;
            let row_aliases = aliases.remove(&id_str).unwrap_or_default();
            let c = Commodity::builder()
                .id(id)
                .code(r.get::<String, _>("code"))
                .maybe_exchange(r.get::<Option<String>, _>("exchange"))
                .maybe_name(r.get::<Option<String>, _>("name"))
                .maybe_description(r.get::<Option<String>, _>("description"))
                .maybe_symbol(r.get::<Option<String>, _>("symbol"))
                .aliases(row_aliases)
                .maybe_active_from(active_from)
                .maybe_active_until(active_until)
                .build();
            out.push(c);
        }
        Ok(out)
    }

    /// Seeds the default currency set when the table is empty (idempotent).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    pub async fn seed_defaults(&self) -> BcResult<()> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM commodities")
            .fetch_one(&self.pool)
            .await?;
        if count > 0 {
            return Ok(());
        }
        for (code, symbol, name, aliases) in DEFAULT_CURRENCIES {
            let c = Commodity::builder()
                .code(*code)
                .symbol(*symbol)
                .name(*name)
                .aliases(aliases.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                .build();
            self.register(&c).await?;
        }
        Ok(())
    }
}

/// Parses an optional `YYYY-MM-DD` column.
fn parse_opt_date(s: Option<String>) -> BcResult<Option<jiff::civil::Date>> {
    s.map(|v| {
        v.parse::<jiff::civil::Date>()
            .map_err(|e| BcError::BadData(format!("invalid date '{v}': {e}")))
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn register_and_list_round_trips_aliases(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let aud = bc_models::Commodity::builder()
            .code("AUD")
            .symbol("A$")
            .name("Australian Dollar")
            .aliases(vec!["AU$".to_owned()])
            .build();
        svc.register(&aud).await.expect("register");
        let all = svc.list_active().await.expect("list");
        let found = all.iter().find(|c| c.code() == "AUD").expect("AUD present");
        assert_eq!(found.symbol(), Some("A$"));
        assert_eq!(found.aliases(), &["AU$".to_owned()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn seed_defaults_is_idempotent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        svc.seed_defaults().await.expect("seed 1");
        let first = svc.list_active().await.expect("list 1").len();
        svc.seed_defaults().await.expect("seed 2");
        let second = svc.list_active().await.expect("list 2").len();
        assert_eq!(first, 9);
        assert_eq!(first, second, "seeding twice must not duplicate");
    }
}
