//! Persisted commodity/currency registry: canonical code, display symbol, and
//! alternate input markers (aliases).

use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::Commodity;
use bc_models::CommodityId;
use sqlx::Row as _;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// `(code, symbol, name, aliases, decimals, is_iso, symbol_after)`.
type DefaultCurrency = (
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
    u8,
    bool,
    bool,
);

/// Default currencies seeded into a fresh database.
///
/// Alias sets are deliberately unambiguous (no marker maps to two currencies).
/// Display metadata mirrors the retired static registry exactly.
const DEFAULT_CURRENCIES: &[DefaultCurrency] = &[
    ("USD", "$", "US Dollar", &["US$"], 2, true, false),
    ("AUD", "A$", "Australian Dollar", &["AU$"], 2, true, false),
    ("EUR", "€", "Euro", &[], 2, true, false),
    ("GBP", "£", "British Pound", &[], 2, true, false),
    ("JPY", "¥", "Japanese Yen", &[], 0, true, false),
    ("KRW", "₩", "Korean Won", &[], 0, true, false),
    ("INR", "₹", "Indian Rupee", &[], 2, true, false),
    ("BTC", "₿", "Bitcoin", &[], 8, false, false),
    ("ETH", "ETH", "Ethereum", &[], 9, false, true),
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
        insert_with(&mut tx, c).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Registers a new commodity after validating it introduces no marker ambiguity.
    ///
    /// # Arguments
    ///
    /// * `c` - The commodity to create.
    ///
    /// # Returns
    ///
    /// The stored commodity (as supplied; its id is authoritative).
    ///
    /// # Errors
    ///
    /// Returns [`BcError::MarkerConflict`] if any marker collides, or [`BcError`]
    /// on database failure.
    pub async fn create(&self, c: &Commodity) -> BcResult<Commodity> {
        let existing = self.list_all().await?;
        check_ambiguity(&existing, c)?;
        let mut tx = self.pool.begin().await?;
        insert_with(&mut tx, c).await?;
        tx.commit().await?;
        Ok(c.clone())
    }

    /// Updates an existing commodity's metadata and aliases (its code is immutable).
    ///
    /// # Arguments
    ///
    /// * `c` - The commodity to update, identified by its id.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the id is unknown, [`BcError::InvalidInput`]
    /// if the code differs from the persisted row, [`BcError::MarkerConflict`] on a
    /// marker collision, or [`BcError`] on database failure.
    pub async fn update(&self, c: &Commodity) -> BcResult<()> {
        let existing = self.list_all().await?;
        let current = existing
            .iter()
            .find(|e| e.id() == c.id())
            .ok_or_else(|| BcError::NotFound(c.id().to_string()))?;
        if current.code() != c.code() {
            return Err(BcError::InvalidInput(format!(
                "commodity code is immutable ({} → {})",
                current.code(),
                c.code()
            )));
        }
        check_ambiguity(&existing, c)?;

        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM commodity_aliases WHERE commodity_id = ?")
            .bind(c.id().to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE commodities SET symbol = ?, decimals = ?, is_iso = ?, symbol_after = ? WHERE id = ?",
        )
        .bind(c.symbol())
        .bind(i64::from(c.decimals()))
        .bind(i64::from(c.is_iso()))
        .bind(i64::from(c.symbol_after()))
        .bind(c.id().to_string())
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
    pub async fn list_all(&self) -> BcResult<Vec<Commodity>> {
        let rows = sqlx::query(
            "SELECT id, code, exchange, name, description, symbol, decimals, is_iso, symbol_after, active_from, active_until \
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
                .decimals(u8::try_from(r.get::<i64, _>("decimals")).unwrap_or(2))
                .is_iso(r.get::<i64, _>("is_iso") != 0)
                .symbol_after(r.get::<i64, _>("symbol_after") != 0)
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
        let mut tx = self.pool.begin().await?;
        for (code, symbol, name, aliases, decimals, is_iso, symbol_after) in DEFAULT_CURRENCIES {
            let c = Commodity::builder()
                .code(*code)
                .symbol(*symbol)
                .name(*name)
                .aliases(aliases.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                .decimals(*decimals)
                .is_iso(*is_iso)
                .symbol_after(*symbol_after)
                .build();
            insert_with(&mut tx, &c).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Inserts a commodity row and its alias rows on the provided transaction.
///
/// Used by both [`Service::register`] (its own transaction) and
/// [`Service::seed_defaults`] (one shared transaction for the whole seed) so a
/// mid-seed failure rolls the entire batch back rather than leaving a partial set.
///
/// # Arguments
///
/// * `tx` - The open transaction to execute the inserts on.
/// * `c` - The commodity to insert.
///
/// # Errors
///
/// Returns [`BcError`] on database failure or an alias position overflow.
async fn insert_with(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>, c: &Commodity) -> BcResult<()> {
    sqlx::query(
        "INSERT INTO commodities (id, code, exchange, name, description, symbol, decimals, is_iso, symbol_after, active_from, active_until) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(c.id().to_string())
    .bind(c.code())
    .bind(c.exchange())
    .bind(c.name())
    .bind(c.description())
    .bind(c.symbol())
    .bind(i64::from(c.decimals()))
    .bind(i64::from(c.is_iso()))
    .bind(i64::from(c.symbol_after()))
    .bind(c.active_from().map(|d| d.to_string()))
    .bind(c.active_until().map(|d| d.to_string()))
    .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Rejects a candidate commodity whose markers (code, symbol, aliases) collide
/// with another commodity's markers, or repeat within the candidate itself.
///
/// Codes match case-insensitively; symbols and aliases match exactly — mirroring
/// the UI resolution helper so the store and the amount parser never disagree.
/// Entries in `existing` sharing the candidate's id are skipped so an update does
/// not conflict with its own persisted markers.
///
/// # Arguments
///
/// * `existing` - The currently registered commodities.
/// * `candidate` - The commodity being created or updated.
///
/// # Errors
///
/// Returns [`BcError::MarkerConflict`] on the first colliding marker.
fn check_ambiguity(existing: &[Commodity], candidate: &Commodity) -> BcResult<()> {
    /// Normalised key for a marker: codes upper-cased, symbols/aliases verbatim.
    fn norm(marker: &str, is_code: bool) -> String {
        if is_code {
            marker.to_uppercase()
        } else {
            marker.to_owned()
        }
    }

    // Build the taken-marker map from every *other* commodity.
    let mut taken: HashMap<String, String> = HashMap::new();
    for c in existing {
        if c.id() == candidate.id() {
            continue;
        }
        taken.insert(norm(c.code(), true), c.code().to_owned());
        if let Some(s) = c.symbol() {
            taken.insert(norm(s, false), c.code().to_owned());
        }
        for a in c.aliases() {
            taken.insert(norm(a, false), c.code().to_owned());
        }
    }

    // Candidate's own markers: (raw, is_code).
    let mut own = vec![(candidate.code().to_owned(), true)];
    if let Some(s) = candidate.symbol() {
        own.push((s.to_owned(), false));
    }
    own.extend(candidate.aliases().iter().map(|a| (a.clone(), false)));

    let mut seen: HashSet<String> = HashSet::new();
    for (raw, is_code) in own {
        let key = norm(&raw, is_code);
        if let Some(existing_code) = taken.get(&key) {
            return Err(BcError::MarkerConflict {
                marker: raw,
                existing: existing_code.clone(),
            });
        }
        if !seen.insert(key) {
            return Err(BcError::MarkerConflict {
                marker: raw,
                existing: candidate.code().to_owned(),
            });
        }
    }
    Ok(())
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
        let all = svc.list_all().await.expect("list");
        let found = all.iter().find(|c| c.code() == "AUD").expect("AUD present");
        assert_eq!(found.symbol(), Some("A$"));
        assert_eq!(found.aliases(), &["AU$".to_owned()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn seed_defaults_is_idempotent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        svc.seed_defaults().await.expect("seed 1");
        let first = svc.list_all().await.expect("list 1").len();
        svc.seed_defaults().await.expect("seed 2");
        let second = svc.list_all().await.expect("list 2").len();
        assert_eq!(first, 9);
        assert_eq!(first, second, "seeding twice must not duplicate");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn display_metadata_round_trips(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let eth = bc_models::Commodity::builder()
            .code("ETH")
            .symbol("ETH")
            .decimals(9)
            .is_iso(false)
            .symbol_after(true)
            .build();
        svc.register(&eth).await.expect("register");
        let all = svc.list_all().await.expect("list");
        let found = all.iter().find(|c| c.code() == "ETH").expect("ETH present");
        assert_eq!(found.decimals(), 9);
        assert!(!found.is_iso());
        assert!(found.symbol_after());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn seed_defaults_carry_display_metadata(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        svc.seed_defaults().await.expect("seed");
        let all = svc.list_all().await.expect("list");
        let jpy = all.iter().find(|c| c.code() == "JPY").expect("JPY");
        assert_eq!(jpy.decimals(), 0);
        assert!(jpy.is_iso());
        let btc = all.iter().find(|c| c.code() == "BTC").expect("BTC");
        assert_eq!(btc.decimals(), 8);
        assert!(!btc.is_iso());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_ambiguous_marker(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        svc.seed_defaults().await.expect("seed");
        let clash = bc_models::Commodity::builder()
            .code("XAU")
            .symbol("$")
            .build();
        let err = svc
            .create(&clash)
            .await
            .expect_err("must conflict with USD $");
        assert!(matches!(err, BcError::MarkerConflict { .. }));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_changes_metadata_but_not_code(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let aud = bc_models::Commodity::builder()
            .code("AUD")
            .symbol("A$")
            .build();
        let stored = svc.create(&aud).await.expect("create");

        let edited = bc_models::Commodity::builder()
            .id(stored.id().clone())
            .code("AUD")
            .symbol("A$")
            .aliases(vec!["AU$".to_owned()])
            .decimals(3)
            .build();
        svc.update(&edited).await.expect("update");
        let found = svc
            .list_all()
            .await
            .expect("list")
            .into_iter()
            .find(|c| c.code() == "AUD")
            .expect("AUD");
        assert_eq!(found.aliases(), &["AU$".to_owned()]);
        assert_eq!(found.decimals(), 3);

        let renamed = bc_models::Commodity::builder()
            .id(stored.id().clone())
            .code("NZD")
            .build();
        let err = svc
            .update(&renamed)
            .await
            .expect_err("code change rejected");
        assert!(matches!(err, BcError::InvalidInput(_)));
    }

    #[test]
    fn ambiguity_detects_cross_and_self_collisions() {
        let usd = Commodity::builder()
            .code("USD")
            .symbol("$")
            .aliases(vec!["US$".to_owned()])
            .build();
        let existing = vec![usd];

        // alias collides with existing symbol
        let clash = Commodity::builder()
            .code("AUD")
            .aliases(vec!["$".to_owned()])
            .build();
        assert!(matches!(
            check_ambiguity(&existing, &clash),
            Err(BcError::MarkerConflict { .. })
        ));

        // code collides case-insensitively with existing code
        let clash_code = Commodity::builder().code("usd").build();
        assert!(matches!(
            check_ambiguity(&existing, &clash_code),
            Err(BcError::MarkerConflict { .. })
        ));

        // internal self-collision (alias equals own symbol)
        let self_clash = Commodity::builder()
            .code("NZD")
            .symbol("N$")
            .aliases(vec!["N$".to_owned()])
            .build();
        assert!(matches!(
            check_ambiguity(&[], &self_clash),
            Err(BcError::MarkerConflict { .. })
        ));

        // no collision
        let ok = Commodity::builder()
            .code("AUD")
            .symbol("A$")
            .aliases(vec!["AU$".to_owned()])
            .build();
        check_ambiguity(&existing, &ok).expect("no collision");
    }

    #[test]
    fn ambiguity_skips_same_id() {
        let usd = Commodity::builder().code("USD").symbol("$").build();
        let existing = vec![usd.clone()];
        // Same id, re-registering its own markers — must not self-conflict.
        check_ambiguity(&existing, &usd).expect("same-id update must not self-conflict");
    }
}
